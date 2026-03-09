use libmorpheus::compositor as compsys;
use libmorpheus::mem;
use core::sync::atomic::{AtomicU32, Ordering};
use crate::islands::{CompState, ChildWindow, CASCADE_STEP, TITLE_H, PANEL_H, MAX_WINDOWS};

// trace the first N ticks. after that, shut up.
static TRACE_TICK: AtomicU32 = AtomicU32::new(0);
const TRACE_LIMIT: u32 = 30;

pub fn update(state: &mut CompState) {
    let tick = TRACE_TICK.fetch_add(1, Ordering::Relaxed);
    let tr = tick < TRACE_LIMIT;

    // ── U0: surface_list ──
    let count = compsys::surface_list(&mut state.surface_buf);

    if tr {
        libmorpheus::println!("U0 t={} count={} (raw=0x{:X})", tick, count, count);
    }

    // if this returns u64::MAX, we aren't registered. but we ARE compd. so this shouldn't happen.
    if count == usize::MAX {
        if tr { libmorpheus::println!("U1 EPERM — returning"); }
        return;
    }

    // ── U2: dump raw surface entries ──
    if tr {
        for i in 0..count.min(MAX_WINDOWS) {
            let e = &state.surface_buf[i];
            libmorpheus::println!(
                "U2 i={} pid={} phys=0x{:X} pages={} w={} h={} stride={} fmt={} dirty={}",
                i, e.pid, e.phys_addr, e.pages, e.width, e.height, e.stride, e.format, e.dirty
            );
        }
    }

    // ── alive bitset ──
    let mut alive_pids = [0u32; MAX_WINDOWS];
    let alive_count = count.min(MAX_WINDOWS);
    for i in 0..alive_count {
        alive_pids[i] = state.surface_buf[i].pid;
    }

    // ── reap dead windows ──
    for i in 0..MAX_WINDOWS {
        let pid = if let Some(ref w) = state.windows[i] { w.pid } else { continue };
        let still_alive = alive_pids[..alive_count].iter().any(|&p| p == pid);
        if !still_alive {
            if tr { libmorpheus::println!("U3 reap slot={} pid={}", i, pid); }
            if let Some(ref w) = state.windows[i] {
                if w.mapped && w.surface_vaddr != 0 && w.surface_pages != 0 {
                    let _ = mem::munmap(w.surface_vaddr, w.surface_pages);
                }
            }
            if state.desktop_idx == Some(i) {
                if tr { libmorpheus::println!("U3 desktop_idx cleared (was slot {})", i); }
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

    // ── iterate surfaces ──
    if tr && count == 0 {
        libmorpheus::println!("U4 count=0 — loop body will not execute");
    }

    for i in 0..count {
        let entry = state.surface_buf[i];
        let pid = entry.pid;

        if tr {
            libmorpheus::println!("U5 loop i={} pid={} w={} h={} pages={} phys=0x{:X}",
                i, pid, entry.width, entry.height, entry.pages, entry.phys_addr);
        }

        // skip entries with pid=0. the kernel zero-inits slots and we don't want ghost windows.
        if pid == 0 {
            if tr { libmorpheus::println!("U5a skip pid=0"); }
            continue;
        }

        // see if we already track this pid
        let slot = state.windows.iter().position(|w| {
            w.as_ref().map(|w| w.pid == pid).unwrap_or(false)
        });

        if let Some(idx) = slot {
            if tr { libmorpheus::println!("U6 already-tracked pid={} slot={}", pid, idx); }

            // already tracked — update surface geometry if mapped
            if let Some(ref mut win) = state.windows[idx] {
                if !win.mapped {
                    if tr { libmorpheus::println!("U6a not-mapped, trying surface_map({})", pid); }
                    match compsys::surface_map(entry.pid) {
                        Ok(ptr) => {
                            if tr { libmorpheus::println!("U6b surface_map OK ptr=0x{:X}", ptr as u64); }
                            win.surface_ptr = ptr as *const u32;
                            win.surface_vaddr = ptr as u64;
                            win.surface_pages = entry.pages;
                            win.src_w = entry.width;
                            win.src_h = entry.height;
                            // kernel puts bytes in SurfaceEntry.stride. divide by 4 for pixels.
                            win.src_stride = (entry.stride / 4).max(entry.width.max(1));
                            win.mapped = true;
                        }
                        Err(e) => {
                            if tr { libmorpheus::println!("U6c surface_map FAIL err=0x{:X}", e); }
                        }
                    }
                } else {
                    if tr { libmorpheus::println!("U6d already-mapped, updating geometry"); }
                    win.src_w = entry.width;
                    win.src_h = entry.height;
                    win.src_stride = (entry.stride / 4).max(entry.width.max(1));
                }
            }
        } else {
            // new window — find empty slot
            let empty = state.windows.iter().position(|w| w.is_none());
            let Some(idx) = empty else {
                if tr { libmorpheus::println!("U7 no empty slot for pid={} — skipping", pid); }
                continue;
            };

            if tr {
                libmorpheus::println!("U8 new pid={} → slot={}, calling surface_map", pid, idx);
            }

            // map surface pages into compd's address space
            let vaddr = match compsys::surface_map(pid) {
                Ok(ptr) => {
                    if tr { libmorpheus::println!("U8a surface_map OK ptr=0x{:X}", ptr as u64); }
                    ptr
                }
                Err(e) => {
                    libmorpheus::println!("U8b surface_map FAIL pid={} err=0x{:X}", pid, e);
                    continue;
                }
            };

            // fullscreen surface = shelld desktop (z_layer 0). no decorations, no cascade.
            // heuristic: same dimensions as the framebuffer and no desktop registered yet.
            let is_desktop = state.desktop_idx.is_none()
                && entry.width == state.fb_w
                && entry.height == state.fb_h;

            if tr {
                libmorpheus::println!(
                    "U9 is_desktop={} desk_idx={} e.w={} fb_w={} e.h={} fb_h={}",
                    is_desktop,
                    if state.desktop_idx.is_some() { 1 } else { 0 },
                    entry.width, state.fb_w, entry.height, state.fb_h
                );
            }

            if is_desktop {
                libmorpheus::println!("U10 desktop registered pid={} slot={} (z0)", pid, idx);
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
                // desktop is not focusable. don't touch state.focused.
            } else {
                if tr { libmorpheus::println!("U10 normal window pid={} slot={} (z1)", pid, idx); }
                // normal window — cascade position, decorations, z_layer 1
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

                // auto-focus new window
                state.focused = Some(idx);
            }
        }
    }

    if tr {
        let tracked = state.windows.iter().filter(|w| w.is_some()).count();
        libmorpheus::println!("U11 leaving: tracked={} desktop_idx={} focused={}",
            tracked,
            state.desktop_idx.map(|i| i as i32).unwrap_or(-1),
            state.focused.map(|i| i as i32).unwrap_or(-1));
    }
}
