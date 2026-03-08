extern crate alloc;

use libmorpheus::{compositor as compsys, hw, io, process};
use crate::islands::{CompState, HitRegion, MouseCapture, TITLE_H, BORDER, PANEL_H, MAX_WINDOWS};
use crate::messages::InputMsg;

const CTRL_BRACKET: u8 = 0x1D; // Ctrl+] — focus cycle scancode

pub fn poll(state: &mut CompState) {
    poll_keyboard(state);
    poll_mouse(state);
}

fn poll_keyboard(state: &mut CompState) {
    let mut kb = [0u8; 32];
    let avail = io::stdin_available();
    let n = if avail > 0 { io::read_stdin(&mut kb) } else { 0 };
    if n == 0 { return; }

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
    if ms.dx == 0 && ms.dy == 0 && ms.buttons == 0 {
        return;
    }

    // clamped. because the mouse delta from the kernel is signed and the universe is cruel.
    let fb_w = state.fb_w as i32;
    let fb_h = state.fb_h as i32;
    state.mouse_x = (state.mouse_x + ms.dx as i32).clamp(0, fb_w - 1);
    state.mouse_y = (state.mouse_y + ms.dy as i32).clamp(0, fb_h - 1);

    // always forward movement to shelld so its cursor position tracks ours.
    // buttons are only forwarded when the click is actually for shelld.
    forward_to_desktop(state, ms.dx, ms.dy, 0);

    let left = (ms.buttons & 1) != 0;
    let left_was = (state.last_buttons & 1) != 0;
    let left_pressed = left && !left_was;
    let left_released = !left && left_was;
    let mut route_to_child = true;

    if left_released {
        state.capture = None;
    }

    if left_pressed {
        // panel area is always handled by shelld — intercept BEFORE window hit-test.
        // this ensures the taskbar is always clickable even with overlapping windows.
        let panel_top = (state.fb_h as i32).saturating_sub(PANEL_H as i32);
        if state.mouse_y >= panel_top {
            // re-forward with actual button state so shelld registers the click
            forward_to_desktop(state, 0, 0, ms.buttons);
            state.last_buttons = ms.buttons;
            return;
        }

        if let Some((idx, region)) = hit_test(state, state.mouse_x, state.mouse_y) {
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
                            off_x: state.mouse_x - win.x,
                            off_y: state.mouse_y - win.y,
                        });
                    }
                    route_to_child = false;
                }
                HitRegion::Resize => {
                    if let Some(ref win) = state.windows[idx] {
                        state.capture = Some(MouseCapture::Resize {
                            idx,
                            start_mx: state.mouse_x,
                            start_my: state.mouse_y,
                            start_w: win.w,
                            start_h: win.h,
                        });
                    }
                    route_to_child = false;
                }
                HitRegion::Content => {}
            }
        } else {
            // click on empty desktop — forward buttons to shelld
            forward_to_desktop(state, 0, 0, ms.buttons);
            state.last_buttons = ms.buttons;
            return;
        }
    }

    if left {
        if let Some(capture) = state.capture {
            match capture {
                MouseCapture::Move { idx, off_x, off_y } => {
                    if let Some(ref mut win) = state.windows[idx] {
                        let nx = state.mouse_x - off_x;
                        let ny = state.mouse_y - off_y;
                        let max_x = (state.fb_w as i32 - win.w as i32).max(0);
                        let max_y = (state.fb_h as i32 - win.h as i32).max(TITLE_H as i32);
                        win.x = nx.clamp(0, max_x);
                        win.y = ny.clamp(TITLE_H as i32, max_y);
                    }
                    route_to_child = false;
                }
                MouseCapture::Resize { idx, start_mx, start_my, start_w, start_h } => {
                    if let Some(ref mut win) = state.windows[idx] {
                        let dx = state.mouse_x - start_mx;
                        let dy = state.mouse_y - start_my;
                        let max_w = state.fb_w.saturating_sub(win.x.max(0) as u32).max(160);
                        let max_h = state.fb_h.saturating_sub(win.y.max(0) as u32).max(120);
                        let nw = (start_w as i32 + dx).clamp(160, max_w as i32);
                        let nh = (start_h as i32 + dy).clamp(120, max_h as i32);
                        win.w = nw as u32;
                        win.h = nh as u32;
                    }
                    route_to_child = false;
                }
            }
        }
    }

    state.last_buttons = ms.buttons;

    // forward unhandled mouse to focused child
    if route_to_child {
        if let Some(idx) = state.focused {
            if let Some(ref win) = state.windows[idx] {
                let _ = compsys::mouse_forward(win.pid, ms.dx, ms.dy, ms.buttons);
            }
        }
    }
}

/// forward mouse to shelld (desktop surface). always forward so shelld's position tracks ours.
fn forward_to_desktop(state: &CompState, dx: i16, dy: i16, buttons: u8) {
    if let Some(di) = state.desktop_idx {
        if let Some(ref dw) = state.windows[di] {
            let _ = compsys::mouse_forward(dw.pid, dx, dy, buttons);
        }
    }
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
                if win.z_layer != 1 { continue; }

                let outer_x = win.x - BORDER as i32;
                let outer_y = win.y - TITLE_H as i32 - BORDER as i32;
                let outer_w = win.w as i32 + BORDER as i32 * 2;
                let outer_h = win.h as i32 + TITLE_H as i32 + BORDER as i32 * 2;

                if mx < outer_x || mx >= outer_x + outer_w
                    || my < outer_y || my >= outer_y + outer_h
                {
                    continue;
                }

                // close button
                let tb_x = outer_x + BORDER as i32;
                let tb_y = outer_y + BORDER as i32;
                let tb_w = win.w as i32;
                let close_x = tb_x + tb_w - 34;
                let close_w = 30;
                if my >= tb_y && my < tb_y + TITLE_H as i32
                    && mx >= close_x && mx < close_x + close_w
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
                if mx >= win.x && mx < win.x + win.w as i32
                    && my >= win.y && my < win.y + win.h as i32
                {
                    return Some((idx, HitRegion::Content));
                }
            }
        }
    }

    None
}
