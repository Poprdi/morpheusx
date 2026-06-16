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

/// A keyboard edit to the overview grid, captured inside the decode closure and applied after the
/// batch (the closure can't touch `state` directly — `kbd`/`keymap` are split-borrowed from it).
#[derive(Clone, Copy)]
enum OverviewKey {
    Toggle,
    Exit,
    Nav(wm_geom::Dir),
    Activate,
}

/// A keyboard edit to an open window context menu, captured in the decode closure (which can't touch
/// `state` directly) and applied after the batch.
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

    // Coalesce the rest of this key burst before decoding. The kernel polls PS/2 into the ring
    // on its ~100 Hz timer, so a press's make and the matching break (and the make,make,break,
    // break of a chord) can land in two reads ~10 ms apart. The decoder's modifier look-ahead
    // needs the whole burst in one batch, so accumulate for up to ~2 idle ticks. This only
    // pauses compositing *while keys are arriving* — an idle ring returns above immediately.
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

    // Decoded output can be multi-byte (UTF-8) or escape sequences, so size for the worst case.
    let mut fwd = [0u8; 128];
    let mut fi = 0usize;
    let mut spawn_shell = false;
    let mut show_launcher = false; // Ctrl+Alt+D → defocus so the desktop launcher gets the keyboard.
    let mut cycle_focus: Option<bool> = None; // Some(reverse) when a focus-cycle chord fired.
    let mut wm_cmd: Option<wm_geom::WmCommand> = None;
    let mut overview_act: Option<OverviewKey> = None;
    let mut menu_act: Option<MenuKey> = None;

    // Snapshot the overview flag before the split borrow: while the grid is open it captures the
    // navigation/confirm/cancel keys (and swallows everything else, so no stray bytes reach a client
    // behind the dim).
    let overview_active = state.overview;
    // Likewise the context menu owns the keyboard while it is open (Esc/Enter/arrows drive it,
    // everything else is swallowed).
    let menu_active = state.menu.is_some();

    let CompState { kbd, keymap, .. } = state;
    kbd.feed_batch(keymap, &scan[..total], |key| {
        // The context menu, when open, owns the keyboard before any global hotkey: Esc dismisses,
        // Enter runs the highlighted row, Up/Down move the highlight; any other key is consumed.
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
        // Ctrl+Alt+E toggles the window overview — works to open it AND to dismiss it, so it is
        // checked before the overview's own capture below.
        if key.scancode == SC_E && key.ctrl && key.alt {
            overview_act = Some(OverviewKey::Toggle);
            return;
        }
        // While the overview is open it owns the keyboard: Esc / Enter / arrows drive the grid, and
        // every other key is consumed (never forwarded to a window hidden behind the grid).
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
        // Global window-management hotkeys consume their key (never forwarded to the client).
        if key.scancode == SC_X && key.ctrl && key.alt {
            spawn_shell = true;
            return;
        }
        // Ctrl+Alt+D drops window focus back to the z0 desktop — the keyboard then reaches the
        // launcher, so another app can be opened while windows are already up (without it, every
        // launch focuses its window and the launcher becomes unreachable → only one window ever).
        if key.scancode == SC_D && key.ctrl && key.alt {
            show_launcher = true;
            return;
        }
        // Alt+Tab cycles focus forward, Shift+Alt+Tab reverse — the canonical window switcher.
        if key.scancode == SC_TAB && key.alt {
            cycle_focus = Some(key.shift);
            return;
        }
        if key.scancode == SC_RBRACKET && key.ctrl {
            cycle_focus = Some(false);
            return;
        }
        // Keyboard-first window management (Ctrl+Alt cluster): move/resize/close the focused window.
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
        // Hand the keyboard to the z0 desktop launcher; the open windows stay where they are.
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
        // Keyboard goes to the focused window; with no window focused it falls through to the
        // z0 desktop shell, so the desktop is interactive on its own (the desktop is never
        // "focused" — focus cycles z1 app windows only — yet it must still receive keys).
        let target = state.focused.or(state.desktop_idx);
        if let Some(idx) = target {
            if let Some(ref win) = state.windows[idx] {
                let _ = compsys::forward_input(win.pid, &fwd[..fi]);
            }
        }
    }
}

/// Apply a keyboard window-management command to the focused window. Move/Resize reuse the exact
/// `wm_geom` clamp/snap math the mouse drag uses, so a window ends in an identical valid state
/// however it was driven; Resize then re-notifies the client of its new cell size (as the mouse
/// resize does) so the DE reflows. Close terminates the client — the surface reaper handles cleanup
/// and refocus. A no-op when nothing is focused or the focused surface is the z0 desktop.
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

/// Tile the focused window to a snap zone (Ctrl+Alt + number-row grid). The geometry comes from the
/// host-tested `wm_geom::snap_rect` (work-area aware — it reserves the taskbar), so a keyboard snap
/// lands a window in a rect the mouse/keyboard movers also accept. Maximize (the centre key) is a
/// toggle: it stashes the floating geometry in `saved_rect` and restores it on the next press; any
/// half/quadrant snap is a direct placement and clears that stash. The client is re-notified of its
/// new cell size in every case so the DE reflows to the tiled geometry.
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
                // Already maximized → restore the stashed floating geometry.
                win.x = x;
                win.y = y;
                win.w = w;
                win.h = h;
            } else {
                // Stash the current rect, then fill the work area.
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
        win.saved_rect = None; // a direct tile is not a maximize-restore state.
        win.x = r.x;
        win.y = r.y;
        win.w = r.w as u32;
        win.h = r.h as u32;
        win.mouse_local_valid = false;
    }
    crate::islands::surface_mgr::notify_window_size(state, idx);
}

/// Minimize window `idx`: hide it and hand focus to the next visible window (or none). The same edit
/// the titlebar `[_]` button, the taskbar chip, and the context menu all perform, so they agree on
/// what "minimize" does. The per-frame `publish_window_state` relays it to the shell (which dims the
/// chip); activating the chip restores it.
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

/// Close window `idx`: ask its client to terminate (SIGTERM). The surface reaper handles the cleanup
/// and refocus once the process exits. Shared by the titlebar `[X]`, the Ctrl+Alt+C chord, and the
/// context menu, so "close" means one thing across all three.
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
    // The overview grid suspends all normal window routing/dragging: hover moves the selection, a
    // left press on a thumbnail focuses+raises it and exits (compd draws the cursor itself from
    // mouse_x/y, so no Desktop forward is needed to keep it tracking).
    if state.overview {
        crate::islands::overview::on_mouse(state, &msg);
        state.last_buttons = msg.buttons;
        return;
    }

    // An open context menu owns the pointer: hover highlights a row, a left click runs it, a click
    // outside dismisses. Normal window routing/dragging is suspended while it is up.
    if state.menu.is_some() {
        crate::islands::menu::on_mouse(state, &msg);
        state.last_buttons = msg.buttons;
        return;
    }

    // Keep desktop cursor in sync with absolute position every sample.
    enqueue_mouse_route(state, MouseZRouteMsg::Desktop { buttons: 0 });

    if msg.left_released {
        // A title drag that ended over an edge-snap trigger TILES to that zone instead of dropping
        // the window free where it was dragged (Aero Snap). `apply_snap` reuses the same host-tested
        // `wm_geom::snap_rect` geometry the keyboard tiling uses, so a mouse snap and a Ctrl+Alt snap
        // leave a window in an identical state.
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
            // Log the settled geometry when a free drag ends, so a window's move/resize is observable
            // on the serial console (the compositor has no other window-management readout). Paired
            // with the grab lines below, one line per discrete user action — not per frame.
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

    // Right-press on a window's title bar opens that window's context menu (anchored at the pointer,
    // clamped on screen). The title-band regions all qualify — a right-click over the [_]/[□]/[X]
    // buttons opens the menu rather than firing the button (those are left-click affordances). The
    // menu then owns the pointer (handled by the early return above) until a pick or a dismiss.
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
            // Right-press over the bare desktop (no window under the pointer) → forward to the z0
            // shell WITH the button bits so it opens its own desktop (root) context menu at the
            // cursor. Mirrors the LEFT empty-desktop forward below; without this the press would fall
            // through to `route_to_child` and be delivered to whatever window holds focus instead of
            // the desktop. The shell owns the wallpaper, so its menu is the shell's, not compd's.
            //
            // Clicking the bare desktop also DROPS window focus (standard DE semantics: a click on the
            // root deselects the active window). Critically this is what lets the shell's desktop menu
            // receive the keyboard: keys route to `state.focused.or(desktop_idx)`, so while a window is
            // focused the z0 shell never sees them and the menu's Up/Down/Enter/Esc nav is dead. With
            // focus dropped, the keyboard reaches the shell and drives the menu it just opened.
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
                    // Hide the window and hand the keyboard/focus to the next visible one (or none) —
                    // the same edit the taskbar chip and the context menu take, so they all agree.
                    minimize_window(state, idx);
                    state.capture = None;
                    route_to_child = false;
                },
                HitRegion::Maximize => {
                    // Toggle maximize/restore — the same `saved_rect` toggle the title double-click,
                    // the top-edge Aero snap, and Ctrl+Alt+5 all share, so a window has one restore
                    // path however it was maximized.
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
                    // Double-click the title bar → maximize/restore: a second press on the SAME
                    // window within DOUBLE_CLICK_MS toggles the maximize via `apply_snap`, the same
                    // saved_rect toggle Ctrl+Alt+5 and the top-edge Aero snap use (so however a window
                    // is maximized, one restore path returns it). `is_double_click` is host-tested in
                    // wm_geom. Otherwise begin a normal title-drag Move and record this press as the
                    // potential first click of a pair.
                    let now = libmorpheus::time::uptime_ms();
                    if wm_geom::is_double_click(state.last_title_press, idx, now, DOUBLE_CLICK_MS) {
                        state.last_title_press = None; // consume — a third quick press starts fresh.
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
                        // Pointer minus the grab offset, clamped to keep the window reachable.
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
                    // Aero-snap preview: the edge zone (if any) the live pointer is over. The renderer
                    // highlights it; releasing here tiles the window to it (see the left_released path).
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
                        // Start size + pointer delta, clamped to the fb and snapped to whole cells
                        // (the content is blitted 1:1, so a fractional cell leaves a stale strip).
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
                    // Tell the client its new cell size so it reflows to the dragged geometry.
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

/// Forward mouse to the z0 desktop surface, mapped from the absolute global cursor.
///
/// The desktop is full-fb, so its window-local coordinate IS the absolute fb pixel. Unlike a z1
/// window — whose app treats the forwarded motion as relative and keeps no absolute cursor — the
/// desktop shell integrates these deltas into an absolute pointer to hit-test its launcher and
/// taskbar. So the FIRST forward seeds that pointer with the full absolute position (a delta from
/// the shell's origin of 0,0), not the usual (0,0) baseline: that lands the shell's cursor exactly
/// where this compositor draws the hardware cursor from the very first sample, instead of leaving a
/// fixed offset until the pointer happens to hit a screen edge and both clamp into agreement.
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

    // z0 desktop maps in full-fb desktop-space. z1 windows are blitted 1:1 from the source's
    // top-left, so a window-local coordinate IS the source-local coordinate — no scaling.
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

/// The window region the cursor is hovering (no capture/focus side effects) — drives the cursor
/// shape so the pointer reflects what a click would do (move on the title, resize on the grip).
pub fn hover_region(state: &CompState, mx: i32, my: i32) -> Option<HitRegion> {
    hit_test(state, mx, my).map(|(_, region)| region)
}

pub(crate) fn hit_test(state: &CompState, mx: i32, my: i32) -> Option<(usize, HitRegion)> {
    let mut candidates: [Option<usize>; MAX_WINDOWS] = [None; MAX_WINDOWS];
    let mut cn = 0usize;

    // Focused first, then topmost unfocused (highest index) downward. Minimized windows are hidden,
    // so they are never hit-tested — a click falls through to whatever is visible beneath them.
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
                // The per-window point classification lives in the host-tested `wm_geom` crate so
                // the move/resize/hit math is verifiable off-hardware (no working pointer in QEMU).
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

/// compd's window-chrome metrics as a `wm_geom::Chrome`. The close button is a 30px cell inset 34px
/// from the title bar's right edge (the `[X]`); the rest mirror the module-level chrome constants.
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
