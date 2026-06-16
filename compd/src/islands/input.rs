extern crate alloc;

use crate::islands::{CompState, HitRegion, MouseCapture, BORDER, GRIP, MAX_WINDOWS, TITLE_H};
use crate::messages::{InputMsg, MouseSpatialMsg, MouseZRouteMsg};
use libmorpheus::{compositor as compsys, hw, io, process};

// Raw PS/2 Set 1 scancodes arrive via SYS_KEYBOARD_READ. The decode + modifier state machine
// (resilient to this kernel's make-code corruption — see keymap::ScanDecoder) lives in the
// shared `keymap` crate; compd coalesces each key burst, intercepts its own window-management
// hotkeys, and forwards the decoded bytes to the focused window.
const SC_X: u8 = 0x2D; // Ctrl+Alt+X → spawn /bin/msh.
const SC_RBRACKET: u8 = 0x1B; // Ctrl+(physical ]) → cycle focus.
const SC_TAB: u8 = 0x0F; // Alt+Tab → cycle focus (Shift+Alt+Tab reverses).
const SC_D: u8 = 0x20; // Ctrl+Alt+D → drop focus, hand the keyboard back to the z0 desktop launcher.
const SC_E: u8 = 0x12; // Ctrl+Alt+E → toggle the window overview (Exposé).

// Keys the overview grid captures while it is open. Esc / Ctrl+Alt+E dismiss it; Enter activates the
// selected thumbnail; the arrow cluster (0xE0-prefixed, reported by the decoder on the release edge)
// moves the selection. All are PS/2 Set-1 make codes.
const SC_ESC: u8 = 0x01;
const SC_ENTER: u8 = 0x1C;
const SC_UP: u8 = 0x48;
const SC_DOWN: u8 = 0x50;
const SC_LEFT: u8 = 0x4B;
const SC_RIGHT: u8 = 0x4D;

/// A second title-bar press within this many ms of the first, on the same window, is a double-click
/// → maximize/restore (the saved_rect toggle the Ctrl+Alt+5 / top-edge snap also uses). 400 ms is
/// the conventional desktop double-click window. The predicate is host-tested in `wm_geom`.
const DOUBLE_CLICK_MS: u64 = 400;

/// Aero-snap trigger geometry. A window title dragged within `SNAP_EDGE` px of a screen edge tiles
/// to that edge's zone on release (left/right → halves, top → maximize); the `SNAP_CORNER` band at
/// the work-area top/bottom of a side edge makes it that corner's quadrant instead of a half.
const SNAP_EDGE: i32 = 16;
const SNAP_CORNER: i32 = 160;

/// Deferred overview action (closure can't borrow `state` directly — split borrow with `kbd`/`keymap`).
#[derive(Clone, Copy)]
enum OverviewKey {
    Toggle,
    Exit,
    Nav(wm_geom::Dir),
    Activate,
}

/// Deferred menu action (same split-borrow reason as `OverviewKey`).
#[derive(Clone, Copy)]
enum MenuKey {
    Dismiss,
    Activate,
    Nav(bool), // true = down, false = up.
}

pub fn poll(state: &mut CompState) {
    crate::islands::menu::prune(state);
    poll_keyboard(state);
    poll_mouse(state);
}

fn poll_keyboard(state: &mut CompState) {
    let mut scan = [0u8; 64];
    let mut total = io::read_keyboard(&mut scan);
    if total == 0 {
        return;
    }

    // Coalesce the burst: PS/2 at ~100 Hz may split a chord's make+break across reads.
    // The decoder's modifier look-ahead needs them in one batch; accumulate ≤2 idle ticks.
    let mut idle = 0;
    while total < scan.len() && idle < 2 {
        process::sleep(8);
        let m = io::read_keyboard(&mut scan[total..]);
        if m == 0 {
            idle += 1;
        } else {
            total += m;
            idle = 0;
        }
    }

    let mut fwd = [0u8; 128]; // worst-case multi-byte UTF-8 or escape sequences.
    let mut fi = 0usize;
    let mut spawn_shell = false;
    let mut show_launcher = false; // Ctrl+Alt+D: drop focus so the desktop launcher gets the keyboard.
    let mut cycle_focus: Option<bool> = None; // Some(reverse) when a focus-cycle chord fired.
    let mut wm_cmd: Option<wm_geom::WmCommand> = None;
    let mut overview_act: Option<OverviewKey> = None;
    let mut menu_act: Option<MenuKey> = None;

    // Snapshot before the split borrow of kbd/keymap.
    let overview_active = state.overview;
    let menu_active = state.menu.is_some();

    let CompState { kbd, keymap, .. } = state;
    kbd.feed_batch(keymap, &scan[..total], |key| {
        // Menu owns the keyboard while open; anything other than Esc/Enter/arrows is consumed.
        if menu_active {
            menu_act = Some(match key.scancode {
                SC_ESC => MenuKey::Dismiss,
                SC_ENTER => MenuKey::Activate,
                SC_UP => MenuKey::Nav(false),
                SC_DOWN => MenuKey::Nav(true),
                _ => return,
            });
            return;
        }
        // Ctrl+Alt+E: check before the overview capture so it also dismisses an open grid.
        if key.scancode == SC_E && key.ctrl && key.alt {
            overview_act = Some(OverviewKey::Toggle);
            return;
        }
        // Overview owns the keyboard: Esc/Enter/arrows navigate; everything else is swallowed.
        if overview_active {
            overview_act = Some(match key.scancode {
                SC_ESC => OverviewKey::Exit,
                SC_ENTER => OverviewKey::Activate,
                SC_UP => OverviewKey::Nav(wm_geom::Dir::Up),
                SC_DOWN => OverviewKey::Nav(wm_geom::Dir::Down),
                SC_LEFT => OverviewKey::Nav(wm_geom::Dir::Left),
                SC_RIGHT => OverviewKey::Nav(wm_geom::Dir::Right),
                _ => return, // swallow any other key while the overview is up.
            });
            return;
        }
        if key.scancode == SC_X && key.ctrl && key.alt {
            spawn_shell = true;
            return;
        }
        // Ctrl+Alt+D: without this, the launcher is unreachable once any window holds focus.
        if key.scancode == SC_D && key.ctrl && key.alt {
            show_launcher = true;
            return;
        }
        if key.scancode == SC_TAB && key.alt {
            cycle_focus = Some(key.shift);
            return;
        }
        if key.scancode == SC_RBRACKET && key.ctrl {
            cycle_focus = Some(false);
            return;
        }
        if let Some(cmd) = wm_geom::wm_command(key.scancode, key.ctrl, key.alt, key.shift) {
            wm_cmd = Some(cmd);
            return;
        }
        for &b in key.bytes {
            if fi < fwd.len() {
                fwd[fi] = b;
                fi += 1;
            }
        }
    });

    if let Some(act) = menu_act {
        match act {
            MenuKey::Dismiss => crate::islands::menu::dismiss(state),
            MenuKey::Activate => crate::islands::menu::dispatch_selected(state),
            MenuKey::Nav(down) => crate::islands::menu::nav(state, down),
        }
    }
    if let Some(act) = overview_act {
        match act {
            OverviewKey::Toggle => crate::islands::overview::toggle(state),
            OverviewKey::Exit => state.overview = false,
            OverviewKey::Nav(dir) => crate::islands::overview::nav(state, dir),
            OverviewKey::Activate => crate::islands::overview::activate_selected(state),
        }
    }
    if spawn_shell {
        let _ = process::spawn("/bin/msh");
    }
    if show_launcher {
        state.focused = None;
    }
    if let Some(reverse) = cycle_focus {
        if state
            .ch_input_to_focus
            .send(InputMsg::FocusCycleRequest { reverse })
            .is_err()
        {
            // Drop on full channel; user will press again.
        }
    }
    if let Some(cmd) = wm_cmd {
        apply_wm_command(state, cmd);
    }
    if fi > 0 {
        // Route to focused window, or the z0 desktop if nothing is focused.
        let target = state.focused.or(state.desktop_idx);
        if let Some(idx) = target {
            if let Some(ref win) = state.windows[idx] {
                let _ = compsys::forward_input(win.pid, &fwd[..fi]);
            }
        }
    }
}

/// Apply a keyboard WM command to the focused z1 window; no-op if nothing is focused or z0.
fn apply_wm_command(state: &mut CompState, cmd: wm_geom::WmCommand) {
    let Some(idx) = state.focused else {
        return;
    };
    // Only z1 app windows are managed (the z0 desktop is never focused, but guard regardless).
    if !matches!(state.windows[idx], Some(ref w) if w.z_layer == 1) {
        return;
    }

    if matches!(cmd, wm_geom::WmCommand::Close) {
        close_window(state, idx);
        state.capture = None;
        return;
    }

    if let wm_geom::WmCommand::Snap(zone) = cmd {
        apply_snap(state, idx, zone);
        return;
    }

    let (fb_w, fb_h) = (state.fb_w as i32, state.fb_h as i32);
    let is_resize = matches!(cmd, wm_geom::WmCommand::Resize(_));
    if let Some(ref mut win) = state.windows[idx] {
        let rect = wm_geom::Rect {
            x: win.x,
            y: win.y,
            w: win.w as i32,
            h: win.h as i32,
        };
        let r = wm_geom::apply_command(
            rect,
            cmd,
            fb_w,
            fb_h,
            TITLE_H as i32,
            crate::islands::CELL_W as i32,
            crate::islands::CELL_H as i32,
            160,
            120,
        );
        win.x = r.x;
        win.y = r.y;
        win.w = r.w as u32;
        win.h = r.h as u32;
        win.mouse_local_valid = false;
    }
    if is_resize {
        crate::islands::surface_mgr::notify_window_size(state, idx);
    }
}

/// Tile window `idx` to `zone`. Maximize is a `saved_rect` toggle; other zones clear the stash.
/// Notifies the client of its new cell size in all cases.
pub(crate) fn apply_snap(state: &mut CompState, idx: usize, zone: wm_geom::SnapZone) {
    let (fb_w, fb_h) = (state.fb_w as i32, state.fb_h as i32);
    let rect_for = |zone| {
        wm_geom::snap_rect(
            zone,
            fb_w,
            fb_h,
            crate::islands::PANEL_H as i32,
            TITLE_H as i32,
            BORDER as i32,
            crate::islands::CELL_W as i32,
            crate::islands::CELL_H as i32,
            160,
            120,
        )
    };

    if matches!(zone, wm_geom::SnapZone::Maximize) {
        if let Some(ref mut win) = state.windows[idx] {
            if let Some((x, y, w, h)) = win.saved_rect.take() {
                win.x = x;
                win.y = y;
                win.w = w;
                win.h = h;
            } else {
                win.saved_rect = Some((win.x, win.y, win.w, win.h));
                let r = rect_for(zone);
                win.x = r.x;
                win.y = r.y;
                win.w = r.w as u32;
                win.h = r.h as u32;
            }
            win.mouse_local_valid = false;
        }
        crate::islands::surface_mgr::notify_window_size(state, idx);
        return;
    }

    let r = rect_for(zone);
    if let Some(ref mut win) = state.windows[idx] {
        win.saved_rect = None; // non-maximize snaps clear the restore stash.
        win.x = r.x;
        win.y = r.y;
        win.w = r.w as u32;
        win.h = r.h as u32;
        win.mouse_local_valid = false;
    }
    crate::islands::surface_mgr::notify_window_size(state, idx);
}

/// Hide window `idx` and move focus to the next visible slot (or none).
pub(crate) fn minimize_window(state: &mut CompState, idx: usize) {
    let pid = state.windows[idx].as_ref().map(|w| w.pid).unwrap_or(0);
    if let Some(w) = state.windows[idx].as_mut() {
        w.minimized = true;
    }
    state.focused = wm_geom::next_focus(Some(idx), MAX_WINDOWS, false, |i| {
        crate::islands::focus::focusable(state, i)
    });
    libmorpheus::debug!("minimize win {}", pid);
}

/// SIGTERM the client; the surface reaper handles cleanup and refocus.
pub(crate) fn close_window(state: &mut CompState, idx: usize) {
    if let Some(ref win) = state.windows[idx] {
        let _ = process::kill(win.pid, process::signal::SIGTERM);
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
    let right = (ms.buttons & 2) != 0;
    let right_was = (state.last_buttons & 2) != 0;
    let sample = MouseSpatialMsg {
        mx: state.mouse_x,
        my: state.mouse_y,
        buttons: ms.buttons,
        left_pressed: left && !left_was,
        left_released: !left && left_was,
        right_pressed: right && !right_was,
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
    // Overview suspends normal routing; compd draws its own cursor, no desktop forward needed.
    if state.overview {
        crate::islands::overview::on_mouse(state, &msg);
        state.last_buttons = msg.buttons;
        return;
    }

    // Menu suspends normal routing while open.
    if state.menu.is_some() {
        crate::islands::menu::on_mouse(state, &msg);
        state.last_buttons = msg.buttons;
        return;
    }

    // Toast click is consumed (non-modal hit-test); a miss falls through to normal routing.
    if msg.left_pressed && crate::islands::toasts::dismiss_at(state, msg.mx, msg.my) {
        state.last_buttons = msg.buttons;
        return;
    }

    enqueue_mouse_route(state, MouseZRouteMsg::Desktop { buttons: 0 }); // sync desktop cursor every sample.

    if msg.left_released {
        // A title drag over an edge trigger tiles to that zone (Aero Snap) on release.
        if let (Some(MouseCapture::Move { idx, .. }), Some(zone)) =
            (state.capture, state.snap_preview)
        {
            apply_snap(state, idx, zone);
            if let Some(ref win) = state.windows[idx] {
                libmorpheus::debug!(
                    "snap win {}: {}x{} @ {},{}",
                    win.pid,
                    win.w,
                    win.h,
                    win.x,
                    win.y
                );
            }
        } else if let Some(cap) = state.capture {
            let (MouseCapture::Move { idx, .. } | MouseCapture::Resize { idx, .. }) = cap;
            if let Some(ref win) = state.windows[idx] {
                libmorpheus::debug!(
                    "drop win {}: {}x{} @ {},{}",
                    win.pid,
                    win.w,
                    win.h,
                    win.x,
                    win.y
                );
            }
        }
        state.capture = None;
        state.snap_preview = None;
    }

    // Panel (z3): route to shelld, not the window beneath.
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

    // Right-click on any title-band region opens the context menu (left-click affordances are unaffected).
    if msg.right_pressed {
        if let Some((idx, region)) = hit_test(state, msg.mx, msg.my) {
            if matches!(
                region,
                HitRegion::Title | HitRegion::Close | HitRegion::Minimize | HitRegion::Maximize
            ) {
                state.focused = Some(idx);
                crate::islands::menu::open(state, idx, msg.mx, msg.my);
                state.last_buttons = msg.buttons;
                return;
            }
        } else {
            // Right on bare desktop: drop focus so the shell's menu can receive the keyboard
            // (keys route to focused.or(desktop_idx); a focused window swallows them).
            state.focused = None;
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

    let mut route_to_child = true;

    if msg.left_pressed {
        if let Some((idx, region)) = hit_test(state, msg.mx, msg.my) {
            state.focused = Some(idx);
            match region {
                HitRegion::Close => {
                    close_window(state, idx);
                    state.capture = None;
                    route_to_child = false;
                },
                HitRegion::Minimize => {
                    minimize_window(state, idx);
                    state.capture = None;
                    route_to_child = false;
                },
                HitRegion::Maximize => {
                    apply_snap(state, idx, wm_geom::SnapZone::Maximize);
                    if let Some(ref win) = state.windows[idx] {
                        libmorpheus::debug!(
                            "maximize win {}: {}x{} @ {},{}",
                            win.pid,
                            win.w,
                            win.h,
                            win.x,
                            win.y
                        );
                    }
                    state.capture = None;
                    route_to_child = false;
                },
                HitRegion::Title => {
                    // Double-click (same window within DOUBLE_CLICK_MS) toggles maximize/restore.
                    let now = libmorpheus::time::uptime_ms();
                    if wm_geom::is_double_click(state.last_title_press, idx, now, DOUBLE_CLICK_MS) {
                        state.last_title_press = None; // consume; a third quick press starts a fresh pair.
                        apply_snap(state, idx, wm_geom::SnapZone::Maximize);
                        if let Some(ref win) = state.windows[idx] {
                            libmorpheus::debug!(
                                "dblclick win {}: {}x{} @ {},{}",
                                win.pid,
                                win.w,
                                win.h,
                                win.x,
                                win.y
                            );
                        }
                    } else {
                        state.last_title_press = Some((now, idx));
                        if let Some(ref win) = state.windows[idx] {
                            state.capture = Some(MouseCapture::Move {
                                idx,
                                off_x: msg.mx - win.x,
                                off_y: msg.my - win.y,
                            });
                            libmorpheus::debug!(
                                "grab title: win {} @ {},{}",
                                win.pid,
                                win.x,
                                win.y
                            );
                        }
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
                        libmorpheus::debug!("grab grip: win {} {}x{}", win.pid, win.w, win.h);
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
                    let (fb_w, fb_h) = (state.fb_w as i32, state.fb_h as i32);
                    if let Some(ref mut win) = state.windows[idx] {
                        let (x, y) = wm_geom::clamp_move(
                            win.w as i32,
                            win.h as i32,
                            fb_w,
                            fb_h,
                            TITLE_H as i32,
                            msg.mx - off_x,
                            msg.my - off_y,
                        );
                        win.x = x;
                        win.y = y;
                        win.mouse_local_valid = false;
                    }
                    state.snap_preview = wm_geom::edge_snap_zone(
                        msg.mx,
                        msg.my,
                        fb_w,
                        fb_h,
                        crate::islands::PANEL_H as i32,
                        SNAP_EDGE,
                        SNAP_CORNER,
                    );
                    route_to_child = false;
                },
                MouseCapture::Resize {
                    idx,
                    start_mx,
                    start_my,
                    start_w,
                    start_h,
                } => {
                    let (fb_w, fb_h) = (state.fb_w as i32, state.fb_h as i32);
                    if let Some(ref mut win) = state.windows[idx] {
                        // Snap to whole cells: 1:1 blit leaves a stale strip on a fractional edge.
                        let (w, h) = wm_geom::snap_resize(
                            start_w as i32,
                            start_h as i32,
                            msg.mx - start_mx,
                            msg.my - start_my,
                            win.x,
                            win.y,
                            fb_w,
                            fb_h,
                            crate::islands::CELL_W as i32,
                            crate::islands::CELL_H as i32,
                            160,
                            120,
                        );
                        win.w = w as u32;
                        win.h = h as u32;
                        win.mouse_local_valid = false;
                    }
                    crate::islands::surface_mgr::notify_window_size(state, idx);
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

/// Forward mouse to the z0 desktop. On the first sample, seeds the shell's pointer with the full
/// absolute position so it tracks the compositor cursor from frame one (not from the next screen-edge clamp).
fn forward_to_desktop(state: &mut CompState, buttons: u8) {
    if let Some(di) = state.desktop_idx {
        if let Some(ref mut dw) = state.windows[di] {
            let (local_x, local_y) = map_global_to_local(state.mouse_x, state.mouse_y, dw);
            let (dx, dy) = if dw.mouse_local_valid {
                (local_x - dw.mouse_local_x, local_y - dw.mouse_local_y)
            } else {
                dw.mouse_local_valid = true;
                (local_x, local_y)
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

    // z0: full-fb coordinate space; z1: 1:1 blit so window-local == source-local.
    if win.z_layer == 0 {
        (mx.clamp(0, sw - 1), my.clamp(0, sh - 1))
    } else {
        let ww = win.w.max(1) as i32;
        let wh = win.h.max(1) as i32;
        let rel_x = (mx - win.x).clamp(0, ww - 1).min(sw - 1);
        let rel_y = (my - win.y).clamp(0, wh - 1).min(sh - 1);
        (rel_x, rel_y)
    }
}

/// Hovered region for cursor-shape decisions; no capture or focus side effects.
pub fn hover_region(state: &CompState, mx: i32, my: i32) -> Option<HitRegion> {
    hit_test(state, mx, my).map(|(_, region)| region)
}

pub(crate) fn hit_test(state: &CompState, mx: i32, my: i32) -> Option<(usize, HitRegion)> {
    let mut candidates: [Option<usize>; MAX_WINDOWS] = [None; MAX_WINDOWS];
    let mut cn = 0usize;

    // Test focused first, then topmost unfocused descending; minimized slots are skipped.
    if let Some(fi) = state.focused {
        candidates[cn] = Some(fi);
        cn += 1;
    }
    for (i, w) in state.windows.iter().enumerate().rev() {
        if let Some(ref win) = w {
            // z0 desktop is not hit-testable.
            if win.z_layer == 1 && !win.minimized && state.focused != Some(i) {
                candidates[cn] = Some(i);
                cn += 1;
            }
        }
    }

    for &c in &candidates[..cn] {
        if let Some(idx) = c {
            if let Some(ref win) = state.windows[idx] {
                if win.z_layer != 1 || win.minimized {
                    continue;
                }
                let rect = wm_geom::Rect {
                    x: win.x,
                    y: win.y,
                    w: win.w as i32,
                    h: win.h as i32,
                };
                if let Some(region) = wm_geom::classify(rect, chrome(), mx, my) {
                    return Some((idx, region.into()));
                }
            }
        }
    }

    None
}

/// compd's chrome metrics: close inset 34px from the right edge, 30px wide; 32px button pitch.
#[inline]
pub(crate) fn chrome() -> wm_geom::Chrome {
    wm_geom::Chrome {
        title_h: TITLE_H as i32,
        border: BORDER as i32,
        grip: GRIP as i32,
        close_off: 34,
        close_w: 30,
        btn_pitch: 32,
    }
}
