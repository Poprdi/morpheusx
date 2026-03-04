use super::*;

impl Compositor {
    pub fn compose(&mut self, fb: &Framebuffer) {
        let fb_ptr = fb.as_ptr();

        raw_fill(
            fb_ptr,
            self.fb_stride,
            0,
            0,
            self.fb_w,
            self.fb_h,
            self.pack(DESKTOP_RGB.0, DESKTOP_RGB.1, DESKTOP_RGB.2),
        );

        let mut order = [0u16; MAX_WINDOWS];
        let mut n = 0usize;
        for (i, w) in self.windows.iter().enumerate() {
            if w.is_some() && self.focused != Some(i) {
                order[n] = i as u16;
                n += 1;
            }
        }
        if let Some(fi) = self.focused {
            if self.windows[fi].is_some() {
                order[n] = fi as u16;
                n += 1;
            }
        }

        for &idx in &order[..n] {
            let idx = idx as usize;
            let is_focused = self.focused == Some(idx);
            if let Some(win) = &self.windows[idx] {
                self.draw_window(fb_ptr, win, is_focused);
            }
        }

        self.draw_cursor(fb_ptr);

        for win in self.windows.iter().flatten() {
            let _ = compsys::surface_dirty_clear(win.pid);
        }
    }

    #[inline]
    fn pack(&self, r: u8, g: u8, b: u8) -> u32 {
        if self.is_bgrx {
            (b as u32) | ((g as u32) << 8) | ((r as u32) << 16)
        } else {
            (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
        }
    }

    fn draw_window(&self, fb_ptr: *mut u32, win: &ChildWindow, focused: bool) {
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

        let outer_x = win.x - BORDER as i32;
        let outer_y = win.y - TITLE_H as i32 - BORDER as i32;
        let outer_w = win.w + BORDER * 2;
        let outer_h = win.h + TITLE_H + BORDER * 2;

        self.clip_fill(fb_ptr, outer_x, outer_y, outer_w, BORDER, br, bg, bb);
        self.clip_fill(
            fb_ptr,
            outer_x,
            outer_y + BORDER as i32,
            BORDER,
            outer_h - BORDER,
            br,
            bg,
            bb,
        );
        self.clip_fill(
            fb_ptr,
            outer_x + outer_w as i32 - BORDER as i32,
            outer_y + BORDER as i32,
            BORDER,
            outer_h - BORDER,
            br,
            bg,
            bb,
        );
        self.clip_fill(
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
        self.clip_fill(fb_ptr, tb_x, tb_y, tb_w, TITLE_H, tb_r, tb_g, tb_b);

        let title_str = core::str::from_utf8(&win.title[..win.title_len]).unwrap_or("?");
        let text_x = (tb_x + 6).max(0);
        let text_y = (tb_y + (TITLE_H as i32 - 16) / 2).max(0);
        self.draw_text(
            fb_ptr,
            text_x as u32,
            text_y as u32,
            title_str,
            TITLE_TEXT_RGB,
            (tb_r, tb_g, tb_b),
        );

        let close_x = (tb_x + tb_w as i32 - 32).max(0);
        self.draw_text(
            fb_ptr,
            close_x as u32,
            text_y as u32,
            "[X]",
            TITLE_TEXT_RGB,
            (tb_r, tb_g, tb_b),
        );

        self.clip_fill(
            fb_ptr,
            win.x + win.w as i32 - 12,
            win.y + win.h as i32 - 12,
            12,
            12,
            br,
            bg,
            bb,
        );

        if win.mapped && !win.surface_ptr.is_null() {
            self.blit_surface(fb_ptr, win);
        }
    }

    fn blit_surface(&self, fb_ptr: *mut u32, win: &ChildWindow) {
        let dst_x = win.x;
        let dst_y = win.y;
        let src_w = win.src_w.max(1);
        let src_h = win.src_h.max(1);
        let src_stride = win.src_stride.max(src_w).max(1);
        let dst_w = win.w.max(1);
        let dst_h = win.h.max(1);
        let dst_stride = self.fb_stride;
        let mapped_pixels = (win.surface_pages as usize).saturating_mul(4096 / 4);

        let x0 = dst_x.max(0) as u32;
        let y0 = dst_y.max(0) as u32;
        let x1 = ((dst_x as i64 + dst_w as i64).min(self.fb_w as i64)).max(0) as u32;
        let y1 = ((dst_y as i64 + dst_h as i64).min(self.fb_h as i64)).max(0) as u32;
        if x0 >= x1 || y0 >= y1 {
            return;
        }

        unsafe {
            for dy in y0..y1 {
                let local_y = (dy as i32 - dst_y) as u32;
                let sy = ((local_y as u64 * src_h as u64) / dst_h as u64) as u32;
                let sy = sy.min(src_h.saturating_sub(1));
                let dst_row = dy * dst_stride;
                let src_row = sy * src_stride;

                if src_row as usize >= mapped_pixels {
                    continue;
                }

                for dx in x0..x1 {
                    let local_x = (dx as i32 - dst_x) as u32;
                    let sx = ((local_x as u64 * src_w as u64) / dst_w as u64) as u32;
                    let sx = sx.min(src_w.saturating_sub(1));
                    let src_off = (src_row + sx) as usize;
                    if src_off >= mapped_pixels {
                        continue;
                    }
                    let dst_off = (dst_row + dx) as usize;
                    *fb_ptr.add(dst_off) = *win.surface_ptr.add(src_off);
                }
            }
        }
    }

    fn clip_fill(&self, fb_ptr: *mut u32, x: i32, y: i32, w: u32, h: u32, r: u8, g: u8, b: u8) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = ((x as i64 + w as i64).min(self.fb_w as i64)).max(0) as u32;
        let y1 = ((y as i64 + h as i64).min(self.fb_h as i64)).max(0) as u32;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let px = self.pack(r, g, b);
        raw_fill(fb_ptr, self.fb_stride, x0, y0, x1 - x0, y1 - y0, px);
    }

    fn draw_text(
        &self,
        fb_ptr: *mut u32,
        x: u32,
        y: u32,
        text: &str,
        fg: (u8, u8, u8),
        bg: (u8, u8, u8),
    ) {
        let fg_px = self.pack(fg.0, fg.1, fg.2);
        let bg_px = self.pack(bg.0, bg.1, bg.2);
        let font_w = 8u32;

        for (ci, ch) in text.chars().enumerate() {
            let gx = x + ci as u32 * font_w;
            if gx + font_w > self.fb_w {
                break;
            }
            let glyph = font::get_glyph_or_space(ch);
            raw_glyph(
                fb_ptr,
                self.fb_stride,
                gx,
                y,
                glyph,
                fg_px,
                bg_px,
                self.fb_h,
            );
        }
    }

    fn draw_cursor(&self, fb_ptr: *mut u32) {
        let cx = self.mouse_x;
        let cy = self.mouse_y;
        let px = self.pack(CURSOR_RGB.0, CURSOR_RGB.1, CURSOR_RGB.2);

        for d in -4i32..=4 {
            raw_put(fb_ptr, self.fb_stride, self.fb_w, self.fb_h, cx + d, cy, px);
            raw_put(fb_ptr, self.fb_stride, self.fb_w, self.fb_h, cx, cy + d, px);
        }

        let outline = self.pack(0, 0, 0);
        for d in [-5i32, 5] {
            raw_put(
                fb_ptr,
                self.fb_stride,
                self.fb_w,
                self.fb_h,
                cx + d,
                cy,
                outline,
            );
            raw_put(
                fb_ptr,
                self.fb_stride,
                self.fb_w,
                self.fb_h,
                cx,
                cy + d,
                outline,
            );
        }
    }
}

#[inline]
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

#[inline]
fn raw_put(buf: *mut u32, stride: u32, w: u32, h: u32, x: i32, y: i32, px: u32) {
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
