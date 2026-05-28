extern crate alloc;

use crate::islands::{CompState, HitRegion, MouseCapture, BORDER, MAX_WINDOWS, TITLE_H};
use crate::messages::{InputMsg, MouseSpatialMsg, MouseZRouteMsg};
use libmorpheus::{compositor as compsys, hw, io, process};

// Raw PS/2 Set 1 scancodes arrive via SYS_KEYBOARD_READ (no longer stdin).
// `handle_scancode` runs the modifier state machine, consumes global hotkeys,
// and decodes the rest to UTF-8 bytes via the active layout (`state.keymap`),
// which compd forwards to the focused window.
const EXTENDED_PREFIX: u8 = 0xE0;
const BREAK_FLAG: u8 = 0x80;
const SC_LCTRL: u8 = 0x1D;
const SC_LSHIFT: u8 = 0x2A;
const SC_RSHIFT: u8 = 0x36;
const SC_LALT: u8 = 0x38; // right Alt (= AltGr) when 0xE0-prefixed
const SC_CAPS: u8 = 0x3A;
const SC_X: u8 = 0x2D; // Ctrl+Alt+X → spawn /bin/msh.
const SC_RBRACKET: u8 = 0x1B; // Ctrl+(physical ]) → cycle focus.

enum KeyAction {
    None,
    SpawnShell,
    CycleFocus,
    /// `n` decoded UTF-8 bytes were written into the caller's emit buffer.
    Emit(usize),
}

pub fn poll(state: &mut CompState) {
    poll_keyboard(state);
    poll_mouse(state);
}

fn poll_keyboard(state: &mut CompState) {
    let mut kb = [0u8; 32];
    let n = io::read_keyboard(&mut kb);
    if n == 0 {
        return;
    }

    // Decoded output can be multi-byte (UTF-8), so size for the worst case.
    let mut fwd = [0u8; 128];
    let mut fi = 0usize;

    for &byte in kb.iter().take(n) {
        let mut emit = [0u8; 4];
        match handle_scancode(state, byte, &mut emit) {
            KeyAction::SpawnShell => {
                let _ = process::spawn("/bin/msh");
            },
            KeyAction::CycleFocus => {
                if state
                    .ch_input_to_focus
                    .send(InputMsg::FocusCycleRequest)
                    .is_err()
                {
                    // Drop on full channel; user will press again.
                }
            },
            KeyAction::Emit(len) => {
                for b in emit.iter().take(len) {
                    if fi < fwd.len() {
                        fwd[fi] = *b;
                        fi += 1;
                    }
                }
            },
            KeyAction::None => {},
        }
    }

    if fi > 0 {
        if let Some(focused_idx) = state.focused {
            if let Some(ref win) = state.windows[focused_idx] {
                let _ = compsys::forward_input(win.pid, &fwd[..fi]);
            }
        }
    }
}

/// Feed one raw PS/2 Set 1 byte into compd's modifier state machine. Updates
/// modifier/lock state, consumes global hotkeys on their press edge, and
/// otherwise decodes the key to UTF-8 bytes (written into `emit`) via the
/// active layout. Returns the action for `poll_keyboard` to apply.
fn handle_scancode(state: &mut CompState, byte: u8, emit: &mut [u8; 4]) -> KeyAction {
    if byte == EXTENDED_PREFIX {
        state.kbd_extended = true;
        return KeyAction::None;
    }
    let is_break = (byte & BREAK_FLAG) != 0;
    let make = byte & !BREAK_FLAG;
    let was_extended = state.kbd_extended;
    state.kbd_extended = false;

    if was_extended {
        // 0xE0-prefixed: right Ctrl, right Alt (= AltGr), and nav keys.
        match make {
            SC_LCTRL => state.kbd_ctrl = !is_break,
            SC_LALT => state.kbd_altgr = !is_break,
            _ => {}, // arrows / nav cluster — no character yet
        }
        return KeyAction::None;
    }

    // Modifiers / locks never produce output.
    match make {
        SC_LCTRL => {
            state.kbd_ctrl = !is_break;
            return KeyAction::None;
        },
        SC_LSHIFT | SC_RSHIFT => {
            state.kbd_shift = !is_break;
            return KeyAction::None;
        },
        SC_LALT => {
            state.kbd_alt = !is_break;
            return KeyAction::None;
        },
        SC_CAPS => {
            if !is_break {
                state.kbd_caps = !state.kbd_caps; // toggle on press
            }
            return KeyAction::None;
        },
        _ => {},
    }

    if is_break {
        return KeyAction::None; // characters and hotkeys fire on the press edge only
    }

    // Global hotkeys consume their trigger key (not forwarded / not decoded).
    if make == SC_X && state.kbd_ctrl && state.kbd_alt {
        return KeyAction::SpawnShell;
    }
    if make == SC_RBRACKET && state.kbd_ctrl {
        return KeyAction::CycleFocus;
    }

    // Decode to characters via the active layout.
    let mods = keymap::Mods {
        shift: state.kbd_shift,
        altgr: state.kbd_altgr,
        ctrl: state.kbd_ctrl,
        caps: state.kbd_caps,
    };
    let len = state.keymap.decode(make, &mods, emit);
    if len > 0 {
        KeyAction::Emit(len)
    } else {
        KeyAction::None
    }
}

fn poll_mouse(state: &mut CompState) {
    let ms = hw::mouse_read();
    if ms.dx == 0 && ms.dy == 0 && ms.buttons == state.last_buttons {
        return;
    }

    let fb_w = state.fb_w as i32;
    let fb_h = state.fb_h as i32;
    state.mouse_x = (state.mouse_x + ms.dx as i32).clamp(0, fb_w - 1);
    state.mouse_y = (state.mouse_y + ms.dy as i32).clamp(0, fb_h - 1);

    let left = (ms.buttons & 1) != 0;
    let left_was = (state.last_buttons & 1) != 0;
    let sample = MouseSpatialMsg {
        mx: state.mouse_x,
        my: state.mouse_y,
        buttons: ms.buttons,
        left_pressed: left && !left_was,
        left_released: !left && left_was,
        in_panel: state.mouse_y >= (state.fb_h as i32 - crate::islands::PANEL_H as i32),
    };

    if let Err(msg) = state.ch_mouse_spatial.send(sample) {
        route_mouse_spatial(state, msg);
    }

    while let Some(msg) = state.ch_mouse_spatial.recv() {
        route_mouse_spatial(state, msg);
    }

    while let Some(msg) = state.ch_mouse_route.recv() {
        dispatch_mouse_route(state, msg);
    }
}

fn route_mouse_spatial(state: &mut CompState, msg: MouseSpatialMsg) {
    // Keep desktop cursor in sync with absolute position every sample.
    enqueue_mouse_route(state, MouseZRouteMsg::Desktop { buttons: 0 });

    if msg.left_released {
        state.capture = None;
    }

    // Panel is z3 overlay — input there goes to shelld, not the window beneath.
    if msg.in_panel {
        enqueue_mouse_route(
            state,
            MouseZRouteMsg::Desktop {
                buttons: msg.buttons,
            },
        );
        state.last_buttons = msg.buttons;
        return;
    }

    let mut route_to_child = true;

    if msg.left_pressed {
        if let Some((idx, region)) = hit_test(state, msg.mx, msg.my) {
            state.focused = Some(idx);
            match region {
                HitRegion::Close => {
                    if let Some(ref win) = state.windows[idx] {
                        let _ = process::kill(win.pid, process::signal::SIGTERM);
                    }
                    state.capture = None;
                    route_to_child = false;
                },
                HitRegion::Title => {
                    if let Some(ref win) = state.windows[idx] {
                        state.capture = Some(MouseCapture::Move {
                            idx,
                            off_x: msg.mx - win.x,
                            off_y: msg.my - win.y,
                        });
                    }
                    route_to_child = false;
                },
                HitRegion::Resize => {
                    if let Some(ref win) = state.windows[idx] {
                        state.capture = Some(MouseCapture::Resize {
                            idx,
                            start_mx: msg.mx,
                            start_my: msg.my,
                            start_w: win.w,
                            start_h: win.h,
                        });
                    }
                    route_to_child = false;
                },
                HitRegion::Content => {},
            }
        } else {
            enqueue_mouse_route(
                state,
                MouseZRouteMsg::Desktop {
                    buttons: msg.buttons,
                },
            );
            state.last_buttons = msg.buttons;
            return;
        }
    }

    if (msg.buttons & 1) != 0 {
        if let Some(capture) = state.capture {
            match capture {
                MouseCapture::Move { idx, off_x, off_y } => {
                    if let Some(ref mut win) = state.windows[idx] {
                        let nx = msg.mx - off_x;
                        let ny = msg.my - off_y;
                        let max_x = (state.fb_w as i32 - win.w as i32).max(0);
                        let max_y = (state.fb_h as i32 - win.h as i32).max(TITLE_H as i32);
                        win.x = nx.clamp(0, max_x);
                        win.y = ny.clamp(TITLE_H as i32, max_y);
                        win.mouse_local_valid = false;
                    }
                    route_to_child = false;
                },
                MouseCapture::Resize {
                    idx,
                    start_mx,
                    start_my,
                    start_w,
                    start_h,
                } => {
                    if let Some(ref mut win) = state.windows[idx] {
                        let dx = msg.mx - start_mx;
                        let dy = msg.my - start_my;
                        let max_w = state.fb_w.saturating_sub(win.x.max(0) as u32).max(160);
                        let max_h = state.fb_h.saturating_sub(win.y.max(0) as u32).max(120);
                        let nw = (start_w as i32 + dx).clamp(160, max_w as i32);
                        let nh = (start_h as i32 + dy).clamp(120, max_h as i32);
                        win.w = nw as u32;
                        win.h = nh as u32;
                        win.mouse_local_valid = false;
                    }
                    route_to_child = false;
                },
            }
        }
    }

    if route_to_child {
        if let Some(idx) = state.focused {
            enqueue_mouse_route(
                state,
                MouseZRouteMsg::Child {
                    idx: idx as u8,
                    buttons: msg.buttons,
                },
            );
        } else {
            enqueue_mouse_route(state, MouseZRouteMsg::None);
        }
    }

    state.last_buttons = msg.buttons;
}

#[inline(always)]
fn enqueue_mouse_route(state: &mut CompState, msg: MouseZRouteMsg) {
    if let Err(msg) = state.ch_mouse_route.send(msg) {
        // Channel full: dispatch inline so no route decision is dropped.
        dispatch_mouse_route(state, msg);
    }
}

fn dispatch_mouse_route(state: &mut CompState, msg: MouseZRouteMsg) {
    match msg {
        MouseZRouteMsg::Desktop { buttons } => {
            forward_to_desktop(state, buttons);
        },
        MouseZRouteMsg::Child { idx, buttons } => {
            let idx = idx as usize;
            if let Some(ref mut win) = state.windows[idx] {
                let (local_x, local_y) = map_global_to_local(state.mouse_x, state.mouse_y, win);
                let (dx, dy) = if win.mouse_local_valid {
                    (local_x - win.mouse_local_x, local_y - win.mouse_local_y)
                } else {
                    win.mouse_local_valid = true;
                    (0, 0)
                };
                win.mouse_local_x = local_x;
                win.mouse_local_y = local_y;
                let dx = dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                let dy = dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                let _ = compsys::mouse_forward(win.pid, dx, dy, buttons);
            }
        },
        MouseZRouteMsg::None => {},
    }
}

/// Forward mouse to shelld's desktop surface, mapped from absolute global cursor.
fn forward_to_desktop(state: &mut CompState, buttons: u8) {
    if let Some(di) = state.desktop_idx {
        if let Some(ref mut dw) = state.windows[di] {
            let (local_x, local_y) = map_global_to_local(state.mouse_x, state.mouse_y, dw);
            let (dx, dy) = if dw.mouse_local_valid {
                (local_x - dw.mouse_local_x, local_y - dw.mouse_local_y)
            } else {
                dw.mouse_local_valid = true;
                (0, 0)
            };
            dw.mouse_local_x = local_x;
            dw.mouse_local_y = local_y;
            let dx = dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            let dy = dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            let _ = compsys::mouse_forward(dw.pid, dx, dy, buttons);
        }
    }
}

#[inline(always)]
fn map_global_to_local(mx: i32, my: i32, win: &crate::islands::ChildWindow) -> (i32, i32) {
    let sw = win.src_w.max(1) as i32;
    let sh = win.src_h.max(1) as i32;

    // z0 maps in desktop-space; z1 maps relative to window origin.
    let (rel_x, rel_y, ww, wh) = if win.z_layer == 0 {
        let ww = sw.max(1);
        let wh = sh.max(1);
        (mx.clamp(0, ww - 1), my.clamp(0, wh - 1), ww, wh)
    } else {
        let ww = win.w.max(1) as i32;
        let wh = win.h.max(1) as i32;
        (
            (mx - win.x).clamp(0, ww - 1),
            (my - win.y).clamp(0, wh - 1),
            ww,
            wh,
        )
    };

    let local_x = if ww <= 1 || sw <= 1 {
        0
    } else {
        (rel_x as i64 * (sw - 1) as i64 / (ww - 1) as i64) as i32
    };
    let local_y = if wh <= 1 || sh <= 1 {
        0
    } else {
        (rel_y as i64 * (sh - 1) as i64 / (wh - 1) as i64) as i32
    };

    (local_x, local_y)
}

fn hit_test(state: &CompState, mx: i32, my: i32) -> Option<(usize, HitRegion)> {
    let mut candidates: [Option<usize>; MAX_WINDOWS] = [None; MAX_WINDOWS];
    let mut cn = 0usize;

    // Focused first, then topmost unfocused (highest index) downward.
    if let Some(fi) = state.focused {
        candidates[cn] = Some(fi);
        cn += 1;
    }
    for (i, w) in state.windows.iter().enumerate().rev() {
        if let Some(ref win) = w {
            // z0 desktop is not hit-testable.
            if win.z_layer == 1 && state.focused != Some(i) {
                candidates[cn] = Some(i);
                cn += 1;
            }
        }
    }

    for &c in &candidates[..cn] {
        if let Some(idx) = c {
            if let Some(ref win) = state.windows[idx] {
                if win.z_layer != 1 {
                    continue;
                }

                let outer_x = win.x - BORDER as i32;
                let outer_y = win.y - TITLE_H as i32 - BORDER as i32;
                let outer_w = win.w as i32 + BORDER as i32 * 2;
                let outer_h = win.h as i32 + TITLE_H as i32 + BORDER as i32 * 2;

                if mx < outer_x
                    || mx >= outer_x + outer_w
                    || my < outer_y
                    || my >= outer_y + outer_h
                {
                    continue;
                }

                let tb_x = outer_x + BORDER as i32;
                let tb_y = outer_y + BORDER as i32;
                let tb_w = win.w as i32;
                let close_x = tb_x + tb_w - 34;
                let close_w = 30;
                if my >= tb_y
                    && my < tb_y + TITLE_H as i32
                    && mx >= close_x
                    && mx < close_x + close_w
                {
                    return Some((idx, HitRegion::Close));
                }

                // 14x14 bottom-right resize grip.
                let resize_x = win.x + win.w as i32 - 14;
                let resize_y = win.y + win.h as i32 - 14;
                if mx >= resize_x && my >= resize_y {
                    return Some((idx, HitRegion::Resize));
                }

                if my >= tb_y && my < tb_y + TITLE_H as i32 {
                    return Some((idx, HitRegion::Title));
                }

                if mx >= win.x
                    && mx < win.x + win.w as i32
                    && my >= win.y
                    && my < win.y + win.h as i32
                {
                    return Some((idx, HitRegion::Content));
                }
            }
        }
    }

    None
}
