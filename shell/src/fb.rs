use libmorpheus::hw;

pub struct Framebuffer {
    base: *mut u32,
    pub width: u32,
    pub height: u32,
    stride_px: u32,
    is_bgrx: bool,
}

unsafe impl Send for Framebuffer {}

impl Framebuffer {
    pub fn init() -> Option<Self> {
        let info = hw::fb_info().ok()?;
        let vaddr = hw::fb_map().ok()?;

        Some(Self {
            base: vaddr as *mut u32,
            width: info.width,
            height: info.height,
            stride_px: info.stride / 4,
            is_bgrx: info.format == 1,
        })
    }

    #[inline]
    fn pack(&self, r: u8, g: u8, b: u8) -> u32 {
        if self.is_bgrx {
            (b as u32) | ((g as u32) << 8) | ((r as u32) << 16)
        } else {
            (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
        }
    }

    #[inline]
    pub fn put_pixel(&self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let px = self.pack(r, g, b);
        unsafe {
            let ptr = self.base.add((y * self.stride_px + x) as usize);
            ptr.write_volatile(px);
        }
    }

    pub fn fill_rect(&self, x: u32, y: u32, w: u32, h: u32, r: u8, g: u8, b: u8) {
        let px = self.pack(r, g, b);
        let x1 = x.min(self.width);
        let y1 = y.min(self.height);
        let x2 = (x.saturating_add(w)).min(self.width);
        let y2 = (y.saturating_add(h)).min(self.height);
        for row in y1..y2 {
            let row_base = (row * self.stride_px + x1) as usize;
            for col in 0..(x2 - x1) {
                unsafe {
                    self.base.add(row_base + col as usize).write_volatile(px);
                }
            }
        }
    }

    pub fn clear(&self, r: u8, g: u8, b: u8) {
        self.fill_rect(0, 0, self.width, self.height, r, g, b);
    }

    pub fn draw_glyph(
        &self,
        glyph: &[u8; 16],
        gx: u32,
        gy: u32,
        fg: (u8, u8, u8),
        bg: (u8, u8, u8),
    ) {
        let fg_px = self.pack(fg.0, fg.1, fg.2);
        let bg_px = self.pack(bg.0, bg.1, bg.2);

        for row in 0u32..16 {
            let py = gy + row;
            if py >= self.height {
                break;
            }
            let bits = glyph[row as usize];
            let row_base = (py * self.stride_px + gx) as usize;
            for col in 0u32..8 {
                let px_x = gx + col;
                if px_x >= self.width {
                    break;
                }
                let is_fg = (bits >> (7 - col)) & 1 == 1;
                unsafe {
                    self.base
                        .add(row_base + col as usize)
                        .write_volatile(if is_fg { fg_px } else { bg_px });
                }
            }
        }
    }

    pub fn scroll_up(&self, rows_px: u32, bg_r: u8, bg_g: u8, bg_b: u8) {
        if rows_px == 0 || rows_px >= self.height {
            self.clear(bg_r, bg_g, bg_b);
            return;
        }
        // Copy rows upward
        for y in 0..(self.height - rows_px) {
            let dst_off = (y * self.stride_px) as usize;
            let src_off = ((y + rows_px) * self.stride_px) as usize;
            for x in 0..self.width as usize {
                unsafe {
                    let val = self.base.add(src_off + x).read_volatile();
                    self.base.add(dst_off + x).write_volatile(val);
                }
            }
        }
        // Clear the vacated bottom region
        let clear_y = self.height - rows_px;
        self.fill_rect(0, clear_y, self.width, rows_px, bg_r, bg_g, bg_b);
    }
}
