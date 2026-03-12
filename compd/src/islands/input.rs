extern crate alloc;

use crate::islands::{CompState, HitRegion, MouseCapture, BORDER, MAX_WINDOWS, TITLE_H};
use crate::messages::{InputMsg, MouseSpatialMsg, MouseZRouteMsg};
use libmorpheus::{compositor as compsys, hw, io, process};

const CTRL_BRACKET: u8 = 0x1D; // Ctrl+] — focus cycle scancode

pub fn poll(state: &mut CompState) {
    poll_keyboard(state);
    poll_mouse(state);
}

fn poll_keyboard(state: &mut CompState) {
    let mut kb = [0u8; 32];
    let avail = io::stdin_available();
    let n = if avail > 0 {
        io::read_stdin(&mut kb)
    } else {
        0
    };
    if n == 0 {
        return;
    }

    let mut has_cycle = false;
    for b in kb.iter_mut().take(n) {
        if *b == CTRL_BRACKET {
            has_cycle = true;
            *b = 0;
        }
    }

    if has_cycle {
        // send focus cycle to focus island via channel
        if let Err(_) = state.ch_input_to_focus.send(InputMsg::FocusCycleRequest) {
            // channel full. focus cycle dropped. user presses again. not the end of the world.
        }
    }

    // forward remaining keyboard bytes to focused window
    let mut fwd = [0u8; 32];
    let mut fi = 0usize;
    for b in kb.iter().take(n) {
        if *b != 0 || !has_cycle {
            fwd[fi] = *b;
            fi += 1;
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

fn poll_mouse(state: &mut CompState) {
    let ms = hw::mouse_read();
    if ms.dx == 0 && ms.dy == 0 && ms.buttons == state.last_buttons {
        return;
    }

    // clamped. because the mouse delta from the kernel is signed and the universe is cruel.
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
    // always keep desktop cursor in sync with absolute position.
    enqueue_mouse_route(state, MouseZRouteMsg::Desktop { buttons: 0 });

    if msg.left_released {
        state.capture = None;
    }

    // panel is visually over windows (z3 overlay), so input there belongs to shelld.
    if msg.in_panel {
        enqueue_mouse_route(state, MouseZRouteMsg::Desktop { buttons: msg.buttons });
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
                        let _ = process::kill(win.pid, process::signal::SIGKILL);
                    }
                    state.capture = None;
                    route_to_child = false;
                }
                HitRegion::Title => {
                    if let Some(ref win) = state.windows[idx] {
                        state.capture = Some(MouseCapture::Move {
                            idx,
                            off_x: msg.mx - win.x,
                            off_y: msg.my - win.y,
                        });
                    }
                    route_to_child = false;
                }
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
                }
                HitRegion::Content => {}
            }
        } else {
            enqueue_mouse_route(state, MouseZRouteMsg::Desktop { buttons: msg.buttons });
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
                }
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
                }
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
        // channel full. handle inline so no route decision gets dropped.
        dispatch_mouse_route(state, msg);
    }
}

fn dispatch_mouse_route(state: &mut CompState, msg: MouseZRouteMsg) {
    match msg {
        MouseZRouteMsg::Desktop { buttons } => {
            forward_to_desktop(state, buttons);
        }
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
        }
        MouseZRouteMsg::None => {}
    }
}

/// forward mouse to shelld (desktop surface), derived from absolute global cursor.
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

    // z0 is desktop-space; z1 is window content-space.
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

    // focused window gets priority (checked first)
    if let Some(fi) = state.focused {
        candidates[cn] = Some(fi);
        cn += 1;
    }
    // then all others in reverse order (topmost unfocused = highest index)
    for (i, w) in state.windows.iter().enumerate().rev() {
        if let Some(ref win) = w {
            // only hit-test z_layer 1 windows. desktop (z0) is not a window.
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

                // close button
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

                // resize handle (bottom-right 14x14)
                let resize_x = win.x + win.w as i32 - 14;
                let resize_y = win.y + win.h as i32 - 14;
                if mx >= resize_x && my >= resize_y {
                    return Some((idx, HitRegion::Resize));
                }

                // title bar
                if my >= tb_y && my < tb_y + TITLE_H as i32 {
                    return Some((idx, HitRegion::Title));
                }

                // content area
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
