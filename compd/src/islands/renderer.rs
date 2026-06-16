use crate::font;
use crate::islands::*;
use core::sync::atomic::{AtomicBool, Ordering};
use libmorpheus::compositor as compsys;
use libmorpheus::info;
static COMPOSE_LOGGED: AtomicBool = AtomicBool::new(false);

/// Painter's algorithm: z0 desktop (shelld fullscreen), z1 app windows
/// (unfocused first, focused last), z3 panel re-blitted on top from the
/// desktop surface so the taskbar stays visible.
pub fn compose(state: &mut CompState) {
    if !COMPOSE_LOGGED.swap(true, Ordering::Relaxed) {
        if state.desktop_idx.is_some() {
            info!("first compose WITH desktop");
        } else {
            info!("first compose NO desktop — solid fill");
        }
    }
    let fb_ptr = state.fb_ptr;

    // z0: desktop or solid fallback.
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
        let (dr, dg, db) = state.desktop_rgb;
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

    // z1: unfocused first, focused last; minimized skipped. Overview suppresses all z1 compositing
    // (compositing full windows behind thumbnails made it indistinguishable from normal mode).
    if !state.overview {
        let mut order = [0u16; MAX_WINDOWS];
        let mut n = 0usize;
        for (i, w) in state.windows.iter().enumerate() {
            if let Some(ref win) = w {
                if win.z_layer == 1 && !win.minimized && state.focused != Some(i) {
                    order[n] = i as u16;
                    n += 1;
                }
            }
        }
        if let Some(fi) = state.focused {
            if let Some(ref win) = state.windows[fi] {
                if win.z_layer == 1 && !win.minimized {
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

        // Aero-snap preview: a translucent tint over the zone a title drag will tile to on release.
        // Drawn above the windows but only over the work area, so the z3 panel below stays clear.
        draw_snap_preview(state, fb_ptr);
    }

    // z3: panel overlay — bottom PANEL_H from desktop, on top of windows.
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

    // Overview: drawn over everything but under the cursor.
    if state.overview {
        draw_overview(state, fb_ptr);
    }

    // Context menu: over everything but the cursor.
    if state.menu.is_some() {
        draw_menu(state, fb_ptr);
    }

    // Toasts: top-right, above everything but the cursor.
    if !state.toasts.is_empty() {
        draw_toasts(state, fb_ptr);
    }

    draw_cursor(state, fb_ptr);

    let _ = libmorpheus::hw::fb_present();

    for win in state.windows.iter().flatten() {
        let _ = compsys::surface_dirty_clear(win.pid);
    }
}

fn draw_window(state: &mut CompState, idx: usize, focused: bool) {
    let fb_ptr = state.fb_ptr;

    // Snapshot before rendering mutates state.
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
        state.title_focused_rgb
    } else {
        TITLE_UNFOCUSED_RGB
    };
    let (br, bg, bb) = if focused {
        state.border_focused_rgb
    } else {
        BORDER_UNFOCUSED_RGB
    };

    let outer_x = win_x - BORDER as i32;
    let outer_y = win_y - TITLE_H as i32 - BORDER as i32;
    let outer_w = win_w + BORDER * 2;
    let outer_h = win_h + TITLE_H + BORDER * 2;

    // Border: top, left, right, bottom.
    clip_fill(state, fb_ptr, outer_x, outer_y, outer_w, BORDER, br, bg, bb);
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

    let tb_x = outer_x + BORDER as i32;
    let tb_y = outer_y + BORDER as i32;
    let tb_w = outer_w - BORDER * 2;
    clip_fill(state, fb_ptr, tb_x, tb_y, tb_w, TITLE_H, tb_r, tb_g, tb_b);

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

    // Controls must match wm_geom::classify exactly (close inset 34, pitch 32, width 30).
    let is_max = state.windows[idx]
        .as_ref()
        .map(|w| w.saved_rect.is_some())
        .unwrap_or(false);
    // `hit_test` respects z-order so an occluded title bar's buttons never falsely highlight.
    let hov_ctl = match crate::islands::input::hit_test(state, state.mouse_x, state.mouse_y) {
        Some((hi, region)) if hi == idx => Some(region),
        _ => None,
    };
    let close_cell_x = tb_x + tb_w as i32 - 34;
    let buttons = [
        (close_cell_x - 64, HitRegion::Minimize, CtlKind::Minimize),
        (
            close_cell_x - 32,
            HitRegion::Maximize,
            if is_max { CtlKind::Restore } else { CtlKind::Maximize },
        ),
        (close_cell_x, HitRegion::Close, CtlKind::Close),
    ];
    for (cell_x, region, kind) in buttons {
        let hovered = hov_ctl == Some(region);
        // Hover wash: close goes the iconic red; the others lighten the title bar a touch.
        if hovered {
            let (hr, hg, hb) = if matches!(region, HitRegion::Close) {
                (200, 70, 70)
            } else {
                (
                    tb_r.saturating_add(38),
                    tb_g.saturating_add(38),
                    tb_b.saturating_add(38),
                )
            };
            clip_fill(state, fb_ptr, cell_x, tb_y, 30, TITLE_H, hr, hg, hb);
        }
        let cell_bg = if hovered && matches!(region, HitRegion::Close) {
            (200, 70, 70)
        } else {
            (tb_r, tb_g, tb_b)
        };
        draw_ctl_mark(state, fb_ptr, cell_x, tb_y, 30, TITLE_H as i32, kind, TITLE_TEXT_RGB, cell_bg);
    }

    if win_mapped && !win_surface_ptr.is_null() {
        blit_1to1(
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

    // Grip drawn AFTER the content blit: painting it before let the 1:1 blit clobber it (was invisible).
    let grip_x = win_x + win_w as i32 - GRIP as i32;
    let grip_y = win_y + win_h as i32 - GRIP as i32;
    clip_fill(state, fb_ptr, grip_x, grip_y, GRIP, GRIP, br, bg, bb);
    let hatch = state.pack(TITLE_TEXT_RGB.0, TITLE_TEXT_RGB.1, TITLE_TEXT_RGB.2);
    for diag in [4i32, 8, 12] {
        for t in 0..diag.min(GRIP as i32 - 2) {
            let px = grip_x + (GRIP as i32 - 2 - t);
            let py = grip_y + (GRIP as i32 - 2 - (diag - 1 - t));
            raw_put(
                fb_ptr,
                state.fb_stride,
                state.fb_w,
                state.fb_h,
                px,
                py,
                hatch,
            );
        }
    }
}

/// 1:1 copy of the source's top-left `dst_w×dst_h` into the window rect, clipped to fb and source.
#[allow(clippy::too_many_arguments)]
fn blit_1to1(
    state: &CompState,
    fb_ptr: *mut u32,
    dst_x: i32,
    dst_y: i32,
    dst_w: u32,
    dst_h: u32,
    src_w: u32,
    src_h: u32,
    src_stride: u32,
    surface_ptr: *const u32,
    surface_pages: u64,
) {
    let copy_w = dst_w.min(src_w);
    let copy_h = dst_h.min(src_h);
    let src_stride = src_stride.max(src_w).max(1);
    let fb_stride = state.fb_stride;
    let mapped_pixels = (surface_pages as usize).saturating_mul(4096 / 4);

    // SAFETY: every dst write is bounds-checked against fb_w/fb_h and fb_stride; every src read is
    // bounds-checked against mapped_pixels (the surface's mapped extent in u32 pixels).
    unsafe {
        for ly in 0..copy_h {
            let dy = dst_y + ly as i32;
            if dy < 0 {
                continue;
            }
            let dy = dy as u32;
            if dy >= state.fb_h {
                break;
            }
            let src_row = (ly * src_stride) as usize;
            let dst_row = (dy * fb_stride) as usize;
            for lx in 0..copy_w {
                let dx = dst_x + lx as i32;
                if dx < 0 {
                    continue;
                }
                let dx = dx as u32;
                if dx >= state.fb_w {
                    break;
                }
                let src_off = src_row + lx as usize;
                if src_off >= mapped_pixels {
                    break;
                }
                *fb_ptr.add(dst_row + dx as usize) = *surface_ptr.add(src_off);
            }
        }
    }
}

/// Bilinear-scale source into dest rect, clipped to fb.
#[allow(clippy::too_many_arguments)]
fn blit_surface(
    state: &CompState,
    fb_ptr: *mut u32,
    dst_x: i32,
    dst_y: i32,
    dst_w: u32,
    dst_h: u32,
    src_w: u32,
    src_h: u32,
    src_stride: u32,
    surface_ptr: *const u32,
    surface_pages: u64,
) {
    let src_w = src_w.max(1);
    let src_h = src_h.max(1);
    let src_stride = src_stride.max(src_w).max(1);
    let dst_w = dst_w.max(1);
    let dst_h = dst_h.max(1);
    let fb_stride = state.fb_stride;
    let mapped_pixels = (surface_pages as usize).saturating_mul(4096 / 4);

    let x0 = dst_x.max(0) as u32;
    let y0 = dst_y.max(0) as u32;
    let x1 = ((dst_x as i64 + dst_w as i64).min(state.fb_w as i64)).max(0) as u32;
    let y1 = ((dst_y as i64 + dst_h as i64).min(state.fb_h as i64)).max(0) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    // 16.16 fixed-point blend, t in [0, 65535].
    #[inline(always)]
    fn lerp8(a: u32, b: u32, t: u32) -> u32 {
        ((a * (65535 - t)) + (b * t)) >> 16
    }

    #[inline(always)]
    fn bilerp(c00: u32, c10: u32, c01: u32, c11: u32, tx: u32, ty: u32) -> u32 {
        let b00 = c00 & 0x0000_00FF;
        let g00 = (c00 >> 8) & 0xFF;
        let r00 = (c00 >> 16) & 0xFF;

        let b10 = c10 & 0x0000_00FF;
        let g10 = (c10 >> 8) & 0xFF;
        let r10 = (c10 >> 16) & 0xFF;

        let b01 = c01 & 0x0000_00FF;
        let g01 = (c01 >> 8) & 0xFF;
        let r01 = (c01 >> 16) & 0xFF;

        let b11 = c11 & 0x0000_00FF;
        let g11 = (c11 >> 8) & 0xFF;
        let r11 = (c11 >> 16) & 0xFF;

        let bx0 = lerp8(b00, b10, tx);
        let gx0 = lerp8(g00, g10, tx);
        let rx0 = lerp8(r00, r10, tx);

        let bx1 = lerp8(b01, b11, tx);
        let gx1 = lerp8(g01, g11, tx);
        let rx1 = lerp8(r01, r11, tx);

        let b = lerp8(bx0, bx1, ty);
        let g = lerp8(gx0, gx1, ty);
        let r = lerp8(rx0, rx1, ty);

        b | (g << 8) | (r << 16)
    }

    // SAFETY: dst and src bounds checked per-pixel against fb_stride and mapped_pixels.
    unsafe {
        for dy in y0..y1 {
            let dst_row = dy * fb_stride;
            let local_y = (dy as i32 - dst_y) as u64;
            let sy_fp = (local_y << 16).saturating_mul(src_h as u64) / dst_h as u64;
            let sy0 = ((sy_fp >> 16) as u32).min(src_h - 1);
            let sy1 = (sy0 + 1).min(src_h - 1);
            let ty = (sy_fp as u32) & 0xFFFF;

            let src_row0 = sy0 * src_stride;
            let src_row1 = sy1 * src_stride;
            if src_row0 as usize >= mapped_pixels || src_row1 as usize >= mapped_pixels {
                continue;
            }

            for dx in x0..x1 {
                let local_x = (dx as i32 - dst_x) as u64;
                let sx_fp = (local_x << 16).saturating_mul(src_w as u64) / dst_w as u64;
                let sx0 = ((sx_fp >> 16) as u32).min(src_w - 1);
                let sx1 = (sx0 + 1).min(src_w - 1);
                let tx = (sx_fp as u32) & 0xFFFF;

                let off00 = (src_row0 + sx0) as usize;
                let off10 = (src_row0 + sx1) as usize;
                let off01 = (src_row1 + sx0) as usize;
                let off11 = (src_row1 + sx1) as usize;
                if off00 >= mapped_pixels
                    || off10 >= mapped_pixels
                    || off01 >= mapped_pixels
                    || off11 >= mapped_pixels
                {
                    continue;
                }

                let c00 = *surface_ptr.add(off00);
                let c10 = *surface_ptr.add(off10);
                let c01 = *surface_ptr.add(off01);
                let c11 = *surface_ptr.add(off11);
                let out = bilerp(c00, c10, c01, c11, tx, ty);

                let dst_off = (dst_row + dx) as usize;
                *fb_ptr.add(dst_off) = out;
            }
        }
    }
}

/// 1:1 copy of a horizontal strip from source to framebuffer. Used for z3 panel overlay.
#[allow(clippy::too_many_arguments)]
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

    // SAFETY: src offsets bounded by mapped_pixels; dst within fb_stride * fb_h.
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

#[allow(clippy::too_many_arguments)]
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

/// The four title-bar control marks. `Maximize`/`Restore` are the two faces of the one button.
#[derive(Clone, Copy)]
enum CtlKind {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Draw one window-control mark centered in its cell using filled rects, not font glyphs.
/// `bg` punches the front square of the restore mark to create the overlapping appearance.
#[allow(clippy::too_many_arguments)]
fn draw_ctl_mark(
    state: &CompState,
    fb_ptr: *mut u32,
    cell_x: i32,
    cell_y: i32,
    cell_w: i32,
    cell_h: i32,
    kind: CtlKind,
    rgb: (u8, u8, u8),
    bg: (u8, u8, u8),
) {
    let (r, g, b) = rgb;
    let cx = cell_x + cell_w / 2;
    let cy = cell_y + cell_h / 2;
    match kind {
        CtlKind::Minimize => {
            clip_fill(state, fb_ptr, cx - 5, cy - 1, 10, 2, r, g, b);
        },
        CtlKind::Maximize => {
            // 10×10 hollow square, 2px stroke.
            clip_fill(state, fb_ptr, cx - 5, cy - 5, 10, 2, r, g, b); // top
            clip_fill(state, fb_ptr, cx - 5, cy + 3, 10, 2, r, g, b); // bottom
            clip_fill(state, fb_ptr, cx - 5, cy - 5, 2, 10, r, g, b); // left
            clip_fill(state, fb_ptr, cx + 3, cy - 5, 2, 10, r, g, b); // right
        },
        CtlKind::Restore => {
            // Two overlapping 8×8 squares: back up-right, front down-left.
            let (bx, by) = (cx - 1, cy - 5);
            clip_fill(state, fb_ptr, bx, by, 8, 2, r, g, b);
            clip_fill(state, fb_ptr, bx, by + 6, 8, 2, r, g, b);
            clip_fill(state, fb_ptr, bx, by, 2, 8, r, g, b);
            clip_fill(state, fb_ptr, bx + 6, by, 2, 8, r, g, b);
            let (fx, fy) = (cx - 5, cy - 1);
            clip_fill(state, fb_ptr, fx, fy, 8, 8, bg.0, bg.1, bg.2);
            clip_fill(state, fb_ptr, fx, fy, 8, 2, r, g, b);
            clip_fill(state, fb_ptr, fx, fy + 6, 8, 2, r, g, b);
            clip_fill(state, fb_ptr, fx, fy, 2, 8, r, g, b);
            clip_fill(state, fb_ptr, fx + 6, fy, 2, 8, r, g, b);
        },
        CtlKind::Close => {
            for i in 0..9 {
                clip_fill(state, fb_ptr, cx - 4 + i, cy - 4 + i, 2, 2, r, g, b);
                clip_fill(state, fb_ptr, cx - 4 + i, cy + 4 - i, 2, 2, r, g, b);
            }
        },
    }
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

/// Translucent accent wash + 2px border over the pending Aero-snap zone during a title drag.
fn draw_snap_preview(state: &CompState, fb_ptr: *mut u32) {
    let Some(zone) = state.snap_preview else {
        return;
    };
    let r = wm_geom::snap_zone_outer(zone, state.fb_w as i32, state.fb_h as i32, PANEL_H as i32);
    let (ar, ag, ab) = state.border_focused_rgb;

    // Translucent fill (~5/16 ≈ 31% accent over the existing scene).
    blend_fill(
        state, fb_ptr, r.x, r.y, r.w as u32, r.h as u32, ar, ag, ab, 5, 16,
    );

    // Solid 2px accent frame.
    const FRAME: u32 = 2;
    clip_fill(state, fb_ptr, r.x, r.y, r.w as u32, FRAME, ar, ag, ab);
    clip_fill(
        state,
        fb_ptr,
        r.x,
        r.y + r.h - FRAME as i32,
        r.w as u32,
        FRAME,
        ar,
        ag,
        ab,
    );
    clip_fill(state, fb_ptr, r.x, r.y, FRAME, r.h as u32, ar, ag, ab);
    clip_fill(
        state,
        fb_ptr,
        r.x + r.w - FRAME as i32,
        r.y,
        FRAME,
        r.h as u32,
        ar,
        ag,
        ab,
    );
}

/// Draw the overview grid: dim the work area, then render each window as a scaled thumbnail with
/// a 1px frame and caption plate; selected thumbnail gets accent frame/plate.
fn draw_overview(state: &CompState, fb_ptr: *mut u32) {
    use crate::islands::overview;
    let area = overview::area(state);

    // 81% dim: wallpaper only (no z1 compositing in overview mode), scrim sinks it behind thumbnails.
    blend_fill(
        state, fb_ptr, area.x, area.y, area.w as u32, area.h as u32, 0, 0, 0, 13, 16,
    );

    let (slots, n) = overview::slots(state);
    if n == 0 {
        // Honest empty state: no windows to lay out. A centred note instead of a blank dim.
        let note = "no open windows";
        let tx = (area.w - note.len() as i32 * 8) / 2;
        let ty = area.y + area.h / 2 - 8;
        draw_text(state, fb_ptr, tx.max(0) as u32, ty.max(0) as u32, note, (170, 170, 180), (10, 10, 16));
        return;
    }

    for (gi, &idx) in slots.iter().enumerate().take(n) {
        let Some(win) = state.windows[idx].as_ref() else {
            continue;
        };
        let cell = wm_geom::overview_cell(gi as u32, n as u32, area, overview::MARGIN, overview::GAP);
        let thumb = wm_geom::overview_thumb(
            cell,
            win.w as i32,
            win.h as i32,
            overview::PAD,
            overview::LABEL_H,
        );
        let selected = state.overview_sel == gi as u32;

        // Scale only the content rect (win.w × win.h), not the full fb-sized surface.
        // The surface is rendered 1:1 into its top-left; the rest is unused black — scaling the full
        // src_w/src_h squished content into the thumbnail corner.
        if win.mapped && !win.surface_ptr.is_null() {
            let content_w = win.w.min(win.src_w);
            let content_h = win.h.min(win.src_h);
            blit_surface(
                state,
                fb_ptr,
                thumb.x,
                thumb.y,
                thumb.w as u32,
                thumb.h as u32,
                content_w,
                content_h,
                win.src_stride,
                win.surface_ptr,
                win.surface_pages,
            );
        }

        // Frame the thumbnail: a bright accent for the selection, a subtle grey otherwise.
        let (fr, fg2, fb2) = if selected {
            state.border_focused_rgb
        } else {
            BORDER_UNFOCUSED_RGB
        };
        let thick = if selected { 2 } else { 1 };
        draw_frame(state, fb_ptr, thumb.x, thumb.y, thumb.w, thumb.h, thick, fr, fg2, fb2);

        // Caption plate matches thumbnail width (not cell width — cell-wide overhangs a narrow thumbnail).
        let plate_rgb = if selected {
            state.title_focused_rgb
        } else {
            (30, 30, 38)
        };
        let plate_x = thumb.x;
        let plate_w = thumb.w.max(1);
        let plate_y = thumb.y + thumb.h + 2;
        clip_fill(
            state,
            fb_ptr,
            plate_x,
            plate_y,
            plate_w as u32,
            overview::LABEL_H as u32,
            plate_rgb.0,
            plate_rgb.1,
            plate_rgb.2,
        );

        let title = core::str::from_utf8(&win.title[..win.title_len]).unwrap_or("?");
        let max_chars = ((plate_w - 8) / 8).max(0) as usize;
        let shown: &str = if title.chars().count() > max_chars {
            let mut end = 0;
            for (k, (bi, _)) in title.char_indices().enumerate() {
                if k >= max_chars {
                    break;
                }
                end = bi + title[bi..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            }
            &title[..end]
        } else {
            title
        };
        let text_w = shown.chars().count() as i32 * 8;
        let text_x = plate_x + (plate_w - text_w) / 2;
        let text_y = plate_y + (overview::LABEL_H - 16) / 2;
        draw_text(
            state,
            fb_ptr,
            text_x.max(cell.x) as u32,
            text_y.max(0) as u32,
            shown,
            TITLE_TEXT_RGB,
            plate_rgb,
        );
    }
}

/// Draw the context menu: drop shadow, dark panel with 1px border, items with hover wash, separators.
fn draw_menu(state: &CompState, fb_ptr: *mut u32) {
    use crate::islands::menu;
    let Some(m) = state.menu.as_ref() else {
        return;
    };

    blend_fill(state, fb_ptr, m.ox + 4, m.oy + 4, m.w as u32, m.h as u32, 0, 0, 0, 7, 16);

    const PANEL: (u8, u8, u8) = (30, 30, 38);
    clip_fill(state, fb_ptr, m.ox, m.oy, m.w as u32, m.h as u32, PANEL.0, PANEL.1, PANEL.2);
    draw_frame(state, fb_ptr, m.ox, m.oy, m.w, m.h, 1, 72, 72, 86);

    let mut y = m.oy;
    for (i, row) in m.rows.iter().enumerate() {
        match row {
            wm_geom::MenuRow::Separator => {
                let ly = y + menu::METRICS.sep_h / 2;
                clip_fill(state, fb_ptr, m.ox + 6, ly, (m.w - 12).max(0) as u32, 1, 60, 60, 72);
                y += menu::METRICS.sep_h;
            },
            wm_geom::MenuRow::Item { action, label } => {
                let rh = menu::METRICS.row_h;
                let selected = m.sel == i;
                let is_close = matches!(action, wm_geom::MenuAction::Close);
                // Danger red for Close so the destructive item reads as such even before it's picked.
                let row_bg = if selected {
                    if is_close {
                        (150, 44, 44)
                    } else {
                        state.title_focused_rgb
                    }
                } else {
                    PANEL
                };
                if selected {
                    clip_fill(
                        state,
                        fb_ptr,
                        m.ox + 1,
                        y,
                        (m.w - 2).max(0) as u32,
                        rh as u32,
                        row_bg.0,
                        row_bg.1,
                        row_bg.2,
                    );
                }
                // Close label stays muted-red at rest; white when highlighted.
                let fg = if selected {
                    (255, 255, 255)
                } else if is_close {
                    (222, 104, 104)
                } else {
                    (220, 220, 228)
                };
                let tx = (m.ox + menu::METRICS.pad_x).max(0) as u32;
                let ty = (y + (rh - 16) / 2).max(0) as u32;
                draw_text(state, fb_ptr, tx, ty, label, fg, row_bg);
                y += rh;
            },
        }
    }
}

/// Draw the toast stack: shadow, dark panel with urgency-coloured left stripe, app name, summary, body.
fn draw_toasts(state: &CompState, fb_ptr: *mut u32) {
    use crate::islands::toasts;
    let (idxs, rects) = toasts::visible(state);
    let hovered = phosphor_notify::layout::toast_hit(&rects, state.mouse_x, state.mouse_y);

    const PANEL: (u8, u8, u8) = (28, 28, 36);
    const PANEL_HOVER: (u8, u8, u8) = (40, 40, 50);
    const SUMMARY_RGB: (u8, u8, u8) = (236, 236, 242);
    const BODY_RGB: (u8, u8, u8) = (176, 176, 188);

    for (k, &i) in idxs.iter().enumerate() {
        let n = &state.toasts[i];
        let r = rects[k];
        let is_hovered = hovered == Some(k);
        let panel = if is_hovered { PANEL_HOVER } else { PANEL };
        let accent = toasts::accent_rgb(state, n.urgency);

        blend_fill(state, fb_ptr, r.x + 4, r.y + 4, r.w as u32, r.h as u32, 0, 0, 0, 6, 16);

        clip_fill(state, fb_ptr, r.x, r.y, r.w as u32, r.h as u32, panel.0, panel.1, panel.2);
        clip_fill(state, fb_ptr, r.x, r.y, toasts::ACCENT_W as u32, r.h as u32, accent.0, accent.1, accent.2);
        draw_frame(state, fb_ptr, r.x, r.y, r.w, r.h, 1, 70, 70, 84);

        // Dismiss `×` in the top-right corner — muted at rest, bright when the toast is hovered.
        let c = phosphor_notify::layout::close_rect(r, toasts::METRICS);
        let x_rgb = if is_hovered { (236, 236, 242) } else { (150, 150, 162) };
        draw_ctl_mark(state, fb_ptr, c.x, c.y, c.w, c.h, CtlKind::Close, x_rgb, panel);

        // Text column: inside the padding, clear of the accent stripe; stop short of the `×`.
        let tx = (r.x + toasts::ACCENT_W + toasts::METRICS.pad).max(0) as u32;
        let mut ty = r.y + toasts::METRICS.pad;
        let line_h = toasts::METRICS.line_h;

        if !n.app.is_empty() {
            draw_text(state, fb_ptr, tx, ty.max(0) as u32, &n.app, accent, panel);
            ty += line_h;
        }
        draw_text(state, fb_ptr, tx, ty.max(0) as u32, &n.summary, SUMMARY_RGB, panel);
        ty += line_h;
        for line in toasts::body_lines(n) {
            draw_text(state, fb_ptr, tx, ty.max(0) as u32, &line, BODY_RGB, panel);
            ty += line_h;
        }
    }
}

/// A `thick`-pixel rectangular frame (top/bottom/left/right) in `(r,g,b)`, clipped to the fb.
#[allow(clippy::too_many_arguments)]
fn draw_frame(
    state: &CompState,
    fb_ptr: *mut u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    thick: u32,
    r: u8,
    g: u8,
    b: u8,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    clip_fill(state, fb_ptr, x, y, w as u32, thick, r, g, b);
    clip_fill(state, fb_ptr, x, y + h - thick as i32, w as u32, thick, r, g, b);
    clip_fill(state, fb_ptr, x, y, thick, h as u32, r, g, b);
    clip_fill(state, fb_ptr, x + w - thick as i32, y, thick, h as u32, r, g, b);
}

/// Alpha-blend a solid colour over the framebuffer rect: each destination pixel moves `num/den` of
/// the way toward `(r,g,b)`. Blends each byte independently, so it is correct for both BGRX and RGBX
/// (the tint is packed in the same order as the destination). Clipped to the framebuffer.
#[allow(clippy::too_many_arguments)]
fn blend_fill(
    state: &CompState,
    fb_ptr: *mut u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    r: u8,
    g: u8,
    b: u8,
    num: u32,
    den: u32,
) {
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = ((x as i64 + w as i64).min(state.fb_w as i64)).max(0) as u32;
    let y1 = ((y as i64 + h as i64).min(state.fb_h as i64)).max(0) as u32;
    if x0 >= x1 || y0 >= y1 || den == 0 {
        return;
    }
    let tint = state.pack(r, g, b);
    let stride = state.fb_stride;
    let inv = den - num;
    // SAFETY: x0..x1 / y0..y1 are clipped to fb bounds, so every offset is inside fb_stride * fb_h.
    unsafe {
        for row in y0..y1 {
            let base = (row * stride) as usize;
            for col in x0..x1 {
                let p = fb_ptr.add(base + col as usize);
                let e = *p;
                let cb = (((e & 0xFF) * inv) + ((tint & 0xFF) * num)) / den;
                let cg = ((((e >> 8) & 0xFF) * inv) + (((tint >> 8) & 0xFF) * num)) / den;
                let cr = ((((e >> 16) & 0xFF) * inv) + (((tint >> 16) & 0xFF) * num)) / den;
                *p = cb | (cg << 8) | (cr << 16);
            }
        }
    }
}

/// Decide the cursor shape (arrow over content/desktop, 4-way move over the title bar or while
/// dragging, diagonal resize over the grip or while resizing) from the live capture and the hovered
/// region. The decision itself is the host-tested `wm_geom::cursor_shape`; here we only translate
/// compd's capture/hit types into its inputs.
fn cursor_mode(state: &CompState) -> wm_geom::CursorShape {
    // Over an open context menu the pointer is just a chooser — keep it an arrow rather than letting
    // the window beneath it (whose chrome the menu may overlap) drive a move/resize cursor.
    if state.menu.is_some() {
        return wm_geom::CursorShape::Arrow;
    }
    // Likewise over a toast: it is a click-to-dismiss target floating above the windows, so an arrow
    // (not a move/resize shape from a window beneath it) is what reads right.
    if !state.toasts.is_empty()
        && crate::islands::toasts::hovering(state, state.mouse_x, state.mouse_y)
    {
        return wm_geom::CursorShape::Arrow;
    }
    let capture = state.capture.map(|c| match c {
        MouseCapture::Move { .. } => wm_geom::Capture::Move,
        MouseCapture::Resize { .. } => wm_geom::Capture::Resize,
    });
    let hover =
        crate::islands::input::hover_region(state, state.mouse_x, state.mouse_y).map(|r| match r {
            HitRegion::Title => wm_geom::Region::Title,
            HitRegion::Close => wm_geom::Region::Close,
            HitRegion::Minimize => wm_geom::Region::Minimize,
            HitRegion::Maximize => wm_geom::Region::Maximize,
            HitRegion::Resize => wm_geom::Region::Resize,
            HitRegion::Content => wm_geom::Region::Content,
        });
    wm_geom::cursor_shape(capture, hover)
}

// Cursor fill masks (`X` = filled pixel). A 1px black outline is generated automatically around the
// union of filled pixels, so only the white interior is hand-drawn. The hotspot — the pixel that
// tracks the true pointer position — differs per shape (top-left tip for the arrow, centre for the
// symmetric move/resize cursors).
const ARROW_MASK: &[&str] = &[
    "X               ",
    "XX              ",
    "XXX             ",
    "XXXX            ",
    "XXXXX           ",
    "XXXXXX          ",
    "XXXXXXX         ",
    "XXXXXXXX        ",
    "XXXXXXXXX       ",
    "XXXXXXXXXX      ",
    "XXXXXXX         ",
    "XXX XXX         ",
    "XX   XXX        ",
    "X     XXX       ",
    "       XXX      ",
    "        XX      ",
];

const MOVE_MASK: &[&str] = &[
    "      X      ",
    "     XXX     ",
    "    XXXXX    ",
    "      X      ",
    "X     X     X",
    "XX    X    XX",
    "XXXXXXXXXXXXX",
    "XX    X    XX",
    "X     X     X",
    "      X      ",
    "    XXXXX    ",
    "     XXX     ",
    "      X      ",
];

fn draw_cursor(state: &CompState, fb_ptr: *mut u32) {
    let mode = cursor_mode(state);
    // A small boolean grid the cursor shape is stamped into; outline + fill are then painted from
    // it in two passes (outline = black ring around the union, fill = white on top).
    const GW: usize = 16;
    const GH: usize = 18;
    let mut grid = [[false; GW]; GH];
    let hotspot: (i32, i32) = match mode {
        wm_geom::CursorShape::Arrow => {
            stamp_mask(&mut grid, ARROW_MASK);
            (0, 0)
        },
        wm_geom::CursorShape::Move => {
            stamp_mask(&mut grid, MOVE_MASK);
            (6, 6)
        },
        wm_geom::CursorShape::Resize => {
            stamp_resize(&mut grid);
            (7, 7)
        },
    };

    let ox = state.mouse_x - hotspot.0;
    let oy = state.mouse_y - hotspot.1;
    let fill = state.pack(CURSOR_RGB.0, CURSOR_RGB.1, CURSOR_RGB.2);
    let outline = state.pack(0, 0, 0);

    // Pass 1: outline — black at the 8-neighbourhood of every filled pixel (overdrawn by fill where
    // the neighbour is itself filled, leaving a clean 1px ring).
    for (gy, row) in grid.iter().enumerate() {
        for (gx, &on) in row.iter().enumerate() {
            if !on {
                continue;
            }
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    raw_put(
                        fb_ptr,
                        state.fb_stride,
                        state.fb_w,
                        state.fb_h,
                        ox + gx as i32 + dx,
                        oy + gy as i32 + dy,
                        outline,
                    );
                }
            }
        }
    }
    // Pass 2: white fill.
    for (gy, row) in grid.iter().enumerate() {
        for (gx, &on) in row.iter().enumerate() {
            if on {
                raw_put(
                    fb_ptr,
                    state.fb_stride,
                    state.fb_w,
                    state.fb_h,
                    ox + gx as i32,
                    oy + gy as i32,
                    fill,
                );
            }
        }
    }
}

/// Stamp an `X`/space mask into the cursor grid (top-left aligned).
fn stamp_mask(grid: &mut [[bool; 16]; 18], mask: &[&str]) {
    for (gy, line) in mask.iter().enumerate() {
        if gy >= grid.len() {
            break;
        }
        for (gx, ch) in line.bytes().enumerate() {
            if gx >= grid[gy].len() {
                break;
            }
            if ch == b'X' {
                grid[gy][gx] = true;
            }
        }
    }
}

/// Procedurally stamp a diagonal (NW↔SE) double-headed resize cursor into the grid: a 2px-thick
/// diagonal spine with an L-shaped arrowhead at each end.
fn stamp_resize(grid: &mut [[bool; 16]; 18]) {
    let mut set = |x: i32, y: i32| {
        if (0..16).contains(&x) && (0..18).contains(&y) {
            grid[y as usize][x as usize] = true;
        }
    };
    // Spine from (2,2) to (12,12), 2px thick.
    for t in 2..=12 {
        set(t, t);
        set(t + 1, t);
    }
    // NW arrowhead (L corner at top-left).
    for k in 0..5 {
        set(2 + k, 2);
        set(2, 2 + k);
    }
    // SE arrowhead (L corner at bottom-right).
    for k in 0..5 {
        set(13 - k, 13);
        set(13, 13 - k);
    }
}

#[inline(always)]
fn raw_fill(buf: *mut u32, stride: u32, x: u32, y: u32, w: u32, h: u32, px: u32) {
    for row in y..y + h {
        let off = (row * stride + x) as usize;
        // SAFETY: caller clips (x, y, w, h) to fb bounds; off + w fits in fb_stride * fb_h.
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
    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
        // SAFETY: bounds checked above.
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
            // SAFETY: caller ensures gx + 8 fits in stride and py < fb_h.
            unsafe {
                buf.add(base + col as usize)
                    .write(if is_fg { fg } else { bg });
            }
        }
    }
}
