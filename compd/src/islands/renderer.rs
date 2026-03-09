use crate::font;
use crate::islands::*;
use core::sync::atomic::{AtomicBool, Ordering};
use libmorpheus::compositor as compsys;
use libmorpheus::io;
static COMPOSE_LOGGED: AtomicBool = AtomicBool::new(false);

/// painter's algorithm with z-layers:
///   z0: desktop background (shelld wallpaper) — fullscreen, no decorations
///   z1: normal app windows — cascaded, decorations, unfocused first + focused last
///   z3: panel overlay — re-blit bottom PANEL_H px from desktop surface above everything
pub fn compose(state: &mut CompState) {
    if !COMPOSE_LOGGED.swap(true, Ordering::Relaxed) {
        if state.desktop_idx.is_some() {
            io::println("compd: first compose WITH desktop");
        } else {
            io::println("compd: first compose NO desktop — solid fill");
        }
    }
    let fb_ptr = state.fb_ptr;

    // --- z-layer 0: desktop background ---
    // if shelld is alive, blit its fullscreen surface. otherwise fall back to solid color.
    let mut drew_desktop = false;
    if let Some(di) = state.desktop_idx {
        if let Some(ref dw) = state.windows[di] {
            if dw.mapped && !dw.surface_ptr.is_null() {
                blit_surface(
                    state,
                    fb_ptr,
                    0,
                    0,
                    state.fb_w,
                    state.fb_h,
                    dw.src_w,
                    dw.src_h,
                    dw.src_stride,
                    dw.surface_ptr,
                    dw.surface_pages,
                );
                drew_desktop = true;
            }
        }
    }
    if !drew_desktop {
        let (dr, dg, db) = DESKTOP_RGB;
        raw_fill(
            fb_ptr,
            state.fb_stride,
            0,
            0,
            state.fb_w,
            state.fb_h,
            state.pack(dr, dg, db),
        );
    }

    // --- z-layer 1: normal app windows ---
    // build z-order: unfocused z1 windows first, focused z1 window last
    let mut order = [0u16; MAX_WINDOWS];
    let mut n = 0usize;
    for (i, w) in state.windows.iter().enumerate() {
        if let Some(ref win) = w {
            if win.z_layer == 1 && state.focused != Some(i) {
                order[n] = i as u16;
                n += 1;
            }
        }
    }
    if let Some(fi) = state.focused {
        if let Some(ref win) = state.windows[fi] {
            if win.z_layer == 1 {
                order[n] = fi as u16;
                n += 1;
            }
        }
    }

    for &idx in &order[..n] {
        let idx = idx as usize;
        let is_focused = state.focused == Some(idx);
        draw_window(state, idx, is_focused);
    }

    // --- z-layer 3: panel overlay ---
    // re-blit bottom PANEL_H pixels from desktop surface OVER windows.
    // ensures the taskbar is always visible even when windows overlap it.
    if let Some(di) = state.desktop_idx {
        if let Some(ref dw) = state.windows[di] {
            if dw.mapped && !dw.surface_ptr.is_null() {
                let panel_y = state.fb_h.saturating_sub(PANEL_H);
                blit_strip(
                    state,
                    fb_ptr,
                    dw.surface_ptr,
                    dw.src_stride,
                    dw.surface_pages,
                    0,
                    panel_y,
                    state.fb_w,
                    PANEL_H,
                );
            }
        }
    }

    draw_cursor(state, fb_ptr);

    // present to hardware
    let _ = libmorpheus::hw::fb_present();

    // clear dirty flags
    for win in state.windows.iter().flatten() {
        let _ = compsys::surface_dirty_clear(win.pid);
    }
}

fn draw_window(state: &mut CompState, idx: usize, focused: bool) {
    let fb_ptr = state.fb_ptr;

    // read all fields we need from the window before doing any rendering
    let (
        win_x,
        win_y,
        win_w,
        win_h,
        win_src_w,
        win_src_h,
        win_src_stride,
        win_mapped,
        win_surface_ptr,
        title_buf,
        title_len,
    ) = {
        let win = state.windows[idx].as_ref().unwrap();
        (
            win.x,
            win.y,
            win.w,
            win.h,
            win.src_w,
            win.src_h,
            win.src_stride,
            win.mapped,
            win.surface_ptr,
            win.title,
            win.title_len,
        )
    };

    let (tb_r, tb_g, tb_b) = if focused {
        TITLE_FOCUSED_RGB
    } else {
        TITLE_UNFOCUSED_RGB
    };
    let (br, bg, bb) = if focused {
        BORDER_FOCUSED_RGB
    } else {
        BORDER_UNFOCUSED_RGB
    };

    let outer_x = win_x - BORDER as i32;
    let outer_y = win_y - TITLE_H as i32 - BORDER as i32;
    let outer_w = win_w + BORDER * 2;
    let outer_h = win_h + TITLE_H + BORDER * 2;

    // border: top
    clip_fill(state, fb_ptr, outer_x, outer_y, outer_w, BORDER, br, bg, bb);
    // border: left
    clip_fill(
        state,
        fb_ptr,
        outer_x,
        outer_y + BORDER as i32,
        BORDER,
        outer_h - BORDER,
        br,
        bg,
        bb,
    );
    // border: right
    clip_fill(
        state,
        fb_ptr,
        outer_x + outer_w as i32 - BORDER as i32,
        outer_y + BORDER as i32,
        BORDER,
        outer_h - BORDER,
        br,
        bg,
        bb,
    );
    // border: bottom
    clip_fill(
        state,
        fb_ptr,
        outer_x,
        outer_y + outer_h as i32 - BORDER as i32,
        outer_w,
        BORDER,
        br,
        bg,
        bb,
    );

    // title bar
    let tb_x = outer_x + BORDER as i32;
    let tb_y = outer_y + BORDER as i32;
    let tb_w = outer_w - BORDER * 2;
    clip_fill(state, fb_ptr, tb_x, tb_y, tb_w, TITLE_H, tb_r, tb_g, tb_b);

    // title text
    let title_str = core::str::from_utf8(&title_buf[..title_len]).unwrap_or("?");
    let text_x = (tb_x + 6).max(0);
    let text_y = (tb_y + (TITLE_H as i32 - 16) / 2).max(0);
    draw_text(
        state,
        fb_ptr,
        text_x as u32,
        text_y as u32,
        title_str,
        TITLE_TEXT_RGB,
        (tb_r, tb_g, tb_b),
    );

    // close button "[X]"
    let close_x = (tb_x + tb_w as i32 - 32).max(0);
    draw_text(
        state,
        fb_ptr,
        close_x as u32,
        text_y as u32,
        "[X]",
        TITLE_TEXT_RGB,
        (tb_r, tb_g, tb_b),
    );

    // resize handle
    clip_fill(
        state,
        fb_ptr,
        win_x + win_w as i32 - 12,
        win_y + win_h as i32 - 12,
        12,
        12,
        br,
        bg,
        bb,
    );

    // blit surface pixels
    if win_mapped && !win_surface_ptr.is_null() {
        blit_surface(
            state,
            fb_ptr,
            win_x,
            win_y,
            win_w,
            win_h,
            win_src_w,
            win_src_h,
            win_src_stride,
            win_surface_ptr,
            state.windows[idx].as_ref().unwrap().surface_pages,
        );
    }
}

/// 1:1 blit from surface buffer to framebuffer. no scaling. clips to both window and FB bounds.
/// the window is a viewport into the top-left of the app's full-resolution surface.
/// apps render at native FB res. we show as many pixels as fit. text stays sharp.
fn blit_surface(
    state: &CompState,
    fb_ptr: *mut u32,
    dst_x: i32,
    dst_y: i32,
    dst_w: u32,
    dst_h: u32,
    _src_w: u32,
    _src_h: u32,
    src_stride: u32,
    surface_ptr: *const u32,
    surface_pages: u64,
) {
    let src_stride = src_stride.max(1);
    let fb_stride = state.fb_stride;
    let mapped_pixels = (surface_pages as usize).saturating_mul(4096 / 4);

    // clip destination rect to framebuffer bounds
    let x0 = dst_x.max(0) as u32;
    let y0 = dst_y.max(0) as u32;
    let x1 = ((dst_x as i64 + dst_w as i64).min(state.fb_w as i64)).max(0) as u32;
    let y1 = ((dst_y as i64 + dst_h as i64).min(state.fb_h as i64)).max(0) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    // source offset: if dst_x < 0 we skip into the source by that many pixels
    let src_x_off = (x0 as i32 - dst_x) as u32;
    let src_y_off = (y0 as i32 - dst_y) as u32;

    // 1:1 copy. one source pixel = one destination pixel. the compiler can auto-vectorize this.
    unsafe {
        for dy in y0..y1 {
            let sy = src_y_off + (dy - y0);
            let dst_row = dy * fb_stride;
            let src_row = sy * src_stride;
            if (src_row + src_x_off) as usize >= mapped_pixels {
                break;
            }

            for dx in x0..x1 {
                let sx = src_x_off + (dx - x0);
                let src_off = (src_row + sx) as usize;
                if src_off >= mapped_pixels {
                    break;
                }
                let dst_off = (dst_row + dx) as usize;
                *fb_ptr.add(dst_off) = *surface_ptr.add(src_off);
            }
        }
    }
}

/// blit a horizontal strip from source surface directly to framebuffer. 1:1 copy, no scaling.
/// used for the panel overlay (z-layer 3) — blit bottom N rows from shelld's surface over windows.
fn blit_strip(
    state: &CompState,
    fb_ptr: *mut u32,
    surface_ptr: *const u32,
    src_stride: u32,
    surface_pages: u64,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) {
    let fb_stride = state.fb_stride;
    let mapped_pixels = (surface_pages as usize).saturating_mul(4096 / 4);
    let x1 = (x + w).min(state.fb_w);
    let y1 = (y + h).min(state.fb_h);

    unsafe {
        for row in y..y1 {
            let src_row_off = (row * src_stride) as usize;
            let dst_row_off = (row * fb_stride) as usize;
            for col in x..x1 {
                let src_off = src_row_off + col as usize;
                if src_off >= mapped_pixels {
                    break;
                }
                let dst_off = dst_row_off + col as usize;
                *fb_ptr.add(dst_off) = *surface_ptr.add(src_off);
            }
        }
    }
}

fn clip_fill(
    state: &CompState,
    fb_ptr: *mut u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    r: u8,
    g: u8,
    b: u8,
) {
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = ((x as i64 + w as i64).min(state.fb_w as i64)).max(0) as u32;
    let y1 = ((y as i64 + h as i64).min(state.fb_h as i64)).max(0) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let px = state.pack(r, g, b);
    raw_fill(fb_ptr, state.fb_stride, x0, y0, x1 - x0, y1 - y0, px);
}

fn draw_text(
    state: &CompState,
    fb_ptr: *mut u32,
    x: u32,
    y: u32,
    text: &str,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
) {
    let fg_px = state.pack(fg.0, fg.1, fg.2);
    let bg_px = state.pack(bg.0, bg.1, bg.2);
    let font_w = 8u32;

    for (ci, ch) in text.chars().enumerate() {
        let gx = x + ci as u32 * font_w;
        if gx + font_w > state.fb_w {
            break;
        }
        let glyph = font::get_glyph_or_space(ch);
        raw_glyph(
            fb_ptr,
            state.fb_stride,
            gx,
            y,
            glyph,
            fg_px,
            bg_px,
            state.fb_h,
        );
    }
}

fn draw_cursor(state: &CompState, fb_ptr: *mut u32) {
    let cx = state.mouse_x;
    let cy = state.mouse_y;
    let px = state.pack(CURSOR_RGB.0, CURSOR_RGB.1, CURSOR_RGB.2);

    // crosshair cursor. 9px arm span.
    for d in -4i32..=4 {
        raw_put(
            fb_ptr,
            state.fb_stride,
            state.fb_w,
            state.fb_h,
            cx + d,
            cy,
            px,
        );
        raw_put(
            fb_ptr,
            state.fb_stride,
            state.fb_w,
            state.fb_h,
            cx,
            cy + d,
            px,
        );
    }

    let outline = state.pack(0, 0, 0);
    for d in [-5i32, 5] {
        raw_put(
            fb_ptr,
            state.fb_stride,
            state.fb_w,
            state.fb_h,
            cx + d,
            cy,
            outline,
        );
        raw_put(
            fb_ptr,
            state.fb_stride,
            state.fb_w,
            state.fb_h,
            cx,
            cy + d,
            outline,
        );
    }
}

// --- raw pixel primitives ---

#[inline(always)]
fn raw_fill(buf: *mut u32, stride: u32, x: u32, y: u32, w: u32, h: u32, px: u32) {
    for row in y..y + h {
        let off = (row * stride + x) as usize;
        unsafe {
            let ptr = buf.add(off);
            for col in 0..w as usize {
                ptr.add(col).write(px);
            }
        }
    }
}

#[inline(always)]
fn raw_put(buf: *mut u32, stride: u32, w: u32, h: u32, x: i32, y: i32, px: u32) {
    // clamped. because the mouse delta from the kernel is signed and the universe is cruel.
    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
        unsafe {
            buf.add((y as u32 * stride + x as u32) as usize).write(px);
        }
    }
}

fn raw_glyph(
    buf: *mut u32,
    stride: u32,
    gx: u32,
    gy: u32,
    glyph: &[u8; 16],
    fg: u32,
    bg: u32,
    fb_h: u32,
) {
    for row in 0u32..16 {
        let py = gy + row;
        if py >= fb_h {
            break;
        }
        let bits = glyph[row as usize];
        let base = (py * stride + gx) as usize;
        for col in 0u32..8 {
            let is_fg = (bits >> (7 - col)) & 1 == 1;
            unsafe {
                buf.add(base + col as usize)
                    .write(if is_fg { fg } else { bg });
            }
        }
    }
}
