use libmorpheus::compositor as compsys;
use libmorpheus::mem;
use crate::islands::{CompState, ChildWindow, CASCADE_STEP, TITLE_H, PANEL_H, MAX_WINDOWS};

pub fn update(state: &mut CompState) {
    let count = compsys::surface_list(&mut state.surface_buf);
    if count == usize::MAX { return; }

    let mut alive_pids = [0u32; MAX_WINDOWS];
    let alive_count = count.min(MAX_WINDOWS);
    for i in 0..alive_count {
        alive_pids[i] = state.surface_buf[i].pid;
    }

    for i in 0..MAX_WINDOWS {
        let pid = if let Some(ref w) = state.windows[i] { w.pid } else { continue };
        let still_alive = alive_pids[..alive_count].iter().any(|&p| p == pid);
        if !still_alive {
            if let Some(ref w) = state.windows[i] {
                if w.mapped && w.surface_vaddr != 0 && w.surface_pages != 0 {
                    let _ = mem::munmap(w.surface_vaddr, w.surface_pages);
                }
            }
            if state.desktop_idx == Some(i) {
                state.desktop_idx = None;
            }
            state.windows[i] = None;
            if state.focused == Some(i) {
                state.focused = state.windows.iter().enumerate().rev()
                    .find(|(_, w)| w.as_ref().map(|w| w.z_layer > 0).unwrap_or(false))
                    .map(|(idx, _)| idx);
            }
        }
    }

    for i in 0..count {
        let entry = state.surface_buf[i];
        let pid = entry.pid;

        if pid == 0 {
            continue;
        }

        // see if we already track this pid
        let slot = state.windows.iter().position(|w| {
            w.as_ref().map(|w| w.pid == pid).unwrap_or(false)
        });

        if let Some(idx) = slot {
            if let Some(ref mut win) = state.windows[idx] {
                if !win.mapped {
                    match compsys::surface_map(entry.pid) {
                        Ok(ptr) => {
                            win.surface_ptr = ptr as *const u32;
                            win.surface_vaddr = ptr as u64;
                            win.surface_pages = entry.pages;
                            win.src_w = entry.width;
                            win.src_h = entry.height;
                            win.src_stride = (entry.stride / 4).max(entry.width.max(1));
                            win.mapped = true;
                        }
                        Err(_) => {}
                    }
                } else {
                    win.src_w = entry.width;
                    win.src_h = entry.height;
                    win.src_stride = (entry.stride / 4).max(entry.width.max(1));
                }
            }
        } else {
            let empty = state.windows.iter().position(|w| w.is_none());
            let Some(idx) = empty else { continue; };

            let vaddr = match compsys::surface_map(pid) {
                Ok(ptr) => ptr,
                Err(_) => continue,
            };

            let is_desktop = state.desktop_idx.is_none()
                && entry.width == state.fb_w
                && entry.height == state.fb_h;

            if is_desktop {
                state.windows[idx] = Some(ChildWindow {
                    pid,
                    surface_ptr:   vaddr as *const u32,
                    mapped:        true,
                    surface_vaddr: vaddr as u64,
                    surface_pages: entry.pages,
                    x:             0,
                    y:             0,
                    w:             state.fb_w,
                    h:             state.fb_h,
                    src_w:         entry.width,
                    src_h:         entry.height,
                    src_stride:    (entry.stride / 4).max(entry.width.max(1)),
                    title:         [0u8; 64],
                    title_len:     0,
                    z_layer:       0,
                });
                state.desktop_idx = Some(idx);
            } else {
                let step = CASCADE_STEP * (state.cascade_n % 5);
                let max_w = state.fb_w.saturating_sub(40);
                let max_h = state.fb_h.saturating_sub(TITLE_H + PANEL_H + 40);
                let w = ((state.fb_w as u64 * 58) / 100) as u32;
                let h = ((state.fb_h as u64 * 58) / 100) as u32;
                let w = w.clamp(320, max_w.max(320));
                let h = h.clamp(220, max_h.max(220));

                let cx = (20 + step).clamp(0, (state.fb_w as i32 - w as i32).max(0));
                let cy = (TITLE_H as i32 + 20 + step)
                    .clamp(TITLE_H as i32, (state.fb_h as i32 - h as i32 - PANEL_H as i32).max(TITLE_H as i32));
                state.cascade_n += 1;

                state.windows[idx] = Some(ChildWindow {
                    pid,
                    surface_ptr:   vaddr as *const u32,
                    mapped:        true,
                    surface_vaddr: vaddr as u64,
                    surface_pages: entry.pages,
                    x:             cx,
                    y:             cy,
                    w,
                    h,
                    src_w:         entry.width,
                    src_h:         entry.height,
                    src_stride:    (entry.stride / 4).max(entry.width.max(1)),
                    title:         [0u8; 64],
                    title_len:     0,
                    z_layer:       1,
                });

                state.focused = Some(idx);
            }
        }
    }
}
