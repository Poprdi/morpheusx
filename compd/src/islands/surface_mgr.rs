use crate::islands::{
    ChildWindow, CompState, CASCADE_STEP, CELL_H, CELL_W, MAX_WINDOWS, PANEL_H, TITLE_H,
};
use libmorpheus::compositor as compsys;
use libmorpheus::mem;

pub fn update(state: &mut CompState) {
    let count = compsys::surface_list(&mut state.surface_buf);
    if count == usize::MAX {
        return;
    }

    let mut alive_pids = [0u32; MAX_WINDOWS];
    let alive_count = count.min(MAX_WINDOWS);
    #[allow(clippy::needless_range_loop)]
    // index drives both alive_pids (write) and surface_buf (read).
    for i in 0..alive_count {
        alive_pids[i] = state.surface_buf[i].pid;
    }

    for i in 0..MAX_WINDOWS {
        let pid = if let Some(ref w) = state.windows[i] {
            w.pid
        } else {
            continue;
        };
        let still_alive = alive_pids[..alive_count].contains(&pid);
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
                // Refocus the topmost remaining *visible* app window (a minimized one is hidden, so
                // it never silently grabs focus when its sibling closes).
                state.focused = state
                    .windows
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, w)| {
                        w.as_ref().map(|w| w.z_layer == 1 && !w.minimized).unwrap_or(false)
                    })
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

        let slot = state
            .windows
            .iter()
            .position(|w| w.as_ref().map(|w| w.pid == pid).unwrap_or(false));

        if let Some(idx) = slot {
            if let Some(ref mut win) = state.windows[idx] {
                if !win.mapped {
                    if let Ok(ptr) = compsys::surface_map(entry.pid) {
                        win.surface_ptr = ptr as *const u32;
                        win.surface_vaddr = ptr as u64;
                        win.surface_pages = entry.pages;
                        win.src_w = entry.width;
                        win.src_h = entry.height;
                        // entry.stride is bytes; blit math wants pixels.
                        win.src_stride = (entry.stride / 4).max(entry.width.max(1));
                        win.mapped = true;
                    }
                } else {
                    win.src_w = entry.width;
                    win.src_h = entry.height;
                    win.src_stride = (entry.stride / 4).max(entry.width.max(1));
                }
            }
        } else {
            let empty = state.windows.iter().position(|w| w.is_none());
            let Some(idx) = empty else {
                continue;
            };

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
                    surface_ptr: vaddr as *const u32,
                    mapped: true,
                    surface_vaddr: vaddr as u64,
                    surface_pages: entry.pages,
                    x: 0,
                    y: 0,
                    w: state.fb_w,
                    h: state.fb_h,
                    src_w: entry.width,
                    src_h: entry.height,
                    src_stride: (entry.stride / 4).max(entry.width.max(1)),
                    mouse_local_x: 0,
                    mouse_local_y: 0,
                    mouse_local_valid: false,
                    title: [0u8; 64],
                    title_len: 0,
                    z_layer: 0,
                    sent_cols: 0,
                    sent_rows: 0,
                    saved_rect: None,
                    minimized: false,
                });
                state.desktop_idx = Some(idx);
            } else {
                let step = CASCADE_STEP * (state.cascade_n % 5);
                // The kernel reports every surface as full-fb-sized, so we can't read a desired
                // window size from `entry`. Pick a real, smaller default window — about 5/8 of the
                // work area — and SNAP it to whole 8×16 cells so the client's grid lands 1:1 with
                // no partial edge cell. Clamped to the work area so the chrome stays reachable.
                let max_w = state.fb_w.saturating_sub(40).max(CELL_W * 24);
                let max_h = state
                    .fb_h
                    .saturating_sub(TITLE_H + PANEL_H + 40)
                    .max(CELL_H * 10);
                let w = snap_cells((state.fb_w * 5 / 8).min(max_w), CELL_W).max(CELL_W * 24);
                let h = snap_cells((state.fb_h * 5 / 8).min(max_h), CELL_H).max(CELL_H * 10);

                let cx = (20 + step).clamp(0, (state.fb_w as i32 - w as i32).max(0));
                let cy = (TITLE_H as i32 + 20 + step).clamp(
                    TITLE_H as i32,
                    (state.fb_h as i32 - h as i32 - PANEL_H as i32).max(TITLE_H as i32),
                );
                state.cascade_n += 1;

                // The client published its window name into the persist store before it mapped this
                // surface (see `platform_morpheusx::run_window`); read it now to label the title bar.
                // The app owns its identity; we own the chrome. Absent ⇒ a blank bar (graceful).
                let (title, title_len) = read_window_title(pid);

                state.windows[idx] = Some(ChildWindow {
                    pid,
                    surface_ptr: vaddr as *const u32,
                    mapped: true,
                    surface_vaddr: vaddr as u64,
                    surface_pages: entry.pages,
                    x: cx,
                    y: cy,
                    w,
                    h,
                    src_w: entry.width,
                    src_h: entry.height,
                    src_stride: (entry.stride / 4).max(entry.width.max(1)),
                    mouse_local_x: 0,
                    mouse_local_y: 0,
                    mouse_local_valid: false,
                    title,
                    title_len,
                    z_layer: 1,
                    sent_cols: 0,
                    sent_rows: 0,
                    saved_rect: None,
                    minimized: false,
                });

                // Seed app-local cursor so the first click lands at the right local pos.
                if let Some(ref mut win) = state.windows[idx] {
                    let (local_x, local_y) =
                        map_global_to_local_spawn(state.mouse_x, state.mouse_y, win);
                    let dx = local_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                    let dy = local_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                    let _ = compsys::mouse_forward(win.pid, dx, dy, 0);
                    win.mouse_local_x = local_x;
                    win.mouse_local_y = local_y;
                    win.mouse_local_valid = true;
                }

                state.focused = Some(idx);
                // Tell the new window its content size in cells so it renders crisply at this
                // geometry (not a full-fb surface scaled down). Bytes queue in its fd-0 ring and
                // are consumed on its first input poll → an `Event::Resize` reflows the DE.
                notify_window_size(state, idx);
            }
        }
    }
}

/// Round a pixel extent DOWN to a whole number of cells. A window's content is blitted 1:1, so
/// its size must be a cell multiple or the client would leave a partial edge cell unpainted.
#[inline(always)]
pub fn snap_cells(px: u32, cell: u32) -> u32 {
    (px / cell.max(1)) * cell.max(1)
}

/// Forward `CSI 8 ; rows ; cols t` to the window's client when its cell size has changed, so the
/// client renders its grid at exactly this geometry. Idempotent: a no-op when the size is unchanged.
pub fn notify_window_size(state: &mut CompState, idx: usize) {
    let (pid, cols, rows) = {
        let Some(ref win) = state.windows[idx] else {
            return;
        };
        let cols = (win.w / CELL_W).clamp(1, u16::MAX as u32) as u16;
        let rows = (win.h / CELL_H).clamp(1, u16::MAX as u32) as u16;
        if cols == win.sent_cols && rows == win.sent_rows {
            return;
        }
        (win.pid, cols, rows)
    };

    let mut buf = [0u8; 24];
    let mut n = 0usize;
    for &b in b"\x1b[8;" {
        buf[n] = b;
        n += 1;
    }
    push_u16(&mut buf, &mut n, rows);
    buf[n] = b';';
    n += 1;
    push_u16(&mut buf, &mut n, cols);
    buf[n] = b't';
    n += 1;

    if compsys::forward_input(pid, &buf[..n]).is_ok() {
        if let Some(ref mut win) = state.windows[idx] {
            win.sent_cols = cols;
            win.sent_rows = rows;
        }
    }
}

/// Append a `u16` as decimal ASCII into `buf` at `*at` (bounded by `buf.len()`).
fn push_u16(buf: &mut [u8], at: &mut usize, mut v: u16) {
    let mut tmp = [0u8; 5];
    let mut k = 0usize;
    if v == 0 {
        tmp[0] = b'0';
        k = 1;
    }
    while v > 0 {
        tmp[k] = b'0' + (v % 10) as u8;
        v /= 10;
        k += 1;
    }
    for i in 0..k {
        if *at < buf.len() {
            buf[*at] = tmp[k - 1 - i];
            *at += 1;
        }
    }
}

/// The persist-store key prefix carrying a window's title-bar name (`de.win.title.<pid>`). The
/// client writes its name there before it maps its surface; we read it here. `de.` is the desktop
/// environment's wire prefix (shared with the launch/desktop-ready gates).
const TITLE_KEY_PREFIX: &[u8] = b"de.win.title.";

/// Read a window's title-bar name from the persist store (`de.win.title.<pid>`). Returns the bytes
/// plus length; a missing key reads as length 0 (graceful — an unlabelled window shows a blank bar).
/// Alloc-free: the key is formatted into a stack buffer, since this runs in the per-frame map walk.
fn read_window_title(pid: u32) -> ([u8; 64], usize) {
    let mut title = [0u8; 64];
    let mut key = [0u8; 32];
    let klen = fmt_title_key(&mut key, pid);
    let Ok(kstr) = core::str::from_utf8(&key[..klen]) else {
        return (title, 0);
    };
    match libmorpheus::persist::get(kstr, &mut title) {
        Ok(n) => (title, n.min(title.len())),
        Err(_) => (title, 0),
    }
}

/// Format `de.win.title.<pid>` (decimal pid) into `buf`, returning the byte length. The 13-byte
/// prefix plus ≤10 digits fits in 32. Alloc-free.
fn fmt_title_key(buf: &mut [u8; 32], pid: u32) -> usize {
    let mut n = 0;
    for &b in TITLE_KEY_PREFIX {
        buf[n] = b;
        n += 1;
    }
    let mut tmp = [0u8; 10];
    let mut k = 0;
    let mut v = pid;
    if v == 0 {
        tmp[0] = b'0';
        k = 1;
    }
    while v > 0 {
        tmp[k] = b'0' + (v % 10) as u8;
        v /= 10;
        k += 1;
    }
    for i in 0..k {
        buf[n] = tmp[k - 1 - i];
        n += 1;
    }
    n
}

/// Window content is blitted 1:1 from the source's top-left, so a window-local coordinate equals
/// the source-local coordinate (no scaling). Used to seed the client's cursor on spawn.
#[inline(always)]
fn map_global_to_local_spawn(mx: i32, my: i32, win: &ChildWindow) -> (i32, i32) {
    let sw = win.src_w.max(1) as i32;
    let sh = win.src_h.max(1) as i32;
    let ww = win.w.max(1) as i32;
    let wh = win.h.max(1) as i32;

    let rel_x = (mx - win.x).clamp(0, ww - 1).min(sw - 1);
    let rel_y = (my - win.y).clamp(0, wh - 1).min(sh - 1);
    (rel_x, rel_y)
}
