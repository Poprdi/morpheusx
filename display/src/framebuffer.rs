//! Framebuffer access via standalone ASM. Each call is a compiler barrier,
//! preventing reordering of hardware writes (same pattern as network/asm).

use crate::asm::fb as asm_fb;
use crate::types::{Color, FramebufferInfo, PixelFormat};

pub struct Framebuffer {
    info: FramebufferInfo,
}

impl Framebuffer {
    /// SAFETY: `info.base` must be valid framebuffer memory for the
    /// lifetime of this struct.
    pub const unsafe fn new(info: FramebufferInfo) -> Self {
        Self { info }
    }

    pub fn info(&self) -> &FramebufferInfo {
        &self.info
    }

    pub fn width(&self) -> u32 {
        self.info.width
    }

    pub fn height(&self) -> u32 {
        self.info.height
    }

    #[inline]
    fn color_to_pixel(&self, color: Color) -> u32 {
        match self.info.format {
            PixelFormat::Bgrx => color.to_bgrx(),
            PixelFormat::Rgbx => color.to_rgbx(),
            _ => color.to_bgrx(),
        }
    }

    /// `info.stride` is bytes, not pixels.
    #[inline]
    fn pixel_addr(&self, x: u32, y: u32) -> u64 {
        self.info.base + (y as u64 * self.info.stride as u64) + (x as u64 * 4)
    }

    #[inline]
    pub fn put_pixel(&mut self, x: u32, y: u32, color: Color) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }

        let pixel_value = self.color_to_pixel(color);
        let addr = self.pixel_addr(x, y);

        // SAFETY: bounds checked above; pixel_addr is within fb.
        unsafe {
            asm_fb::write32(addr, pixel_value);
        }
    }

    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        let x_end = (x + w).min(self.info.width);
        let y_end = (y + h).min(self.info.height);
        let actual_w = x_end.saturating_sub(x);

        if actual_w == 0 {
            return;
        }

        let pixel_value = self.color_to_pixel(color);

        for py in y..y_end {
            let row_addr = self.pixel_addr(x, py);
            // SAFETY: row clipped to fb width; row_addr within fb.
            unsafe {
                asm_fb::memset32(row_addr, pixel_value, actual_w as u64);
            }
        }
    }

    pub fn clear(&mut self, color: Color) {
        let pixel_value = self.color_to_pixel(color);
        let stride = self.info.stride as u64;

        // Row-by-row so stride padding is preserved.
        for y in 0..self.info.height {
            let row_addr = self.info.base + (y as u64 * stride);
            // SAFETY: row addresses are within mapped fb.
            unsafe {
                asm_fb::memset32(row_addr, pixel_value, self.info.width as u64);
            }
        }
    }

    /// Forward copy is safe because dst < src (scrolling up).
    pub fn scroll_up(&mut self, lines: u32, fill_color: Color) {
        if lines == 0 {
            return;
        }
        if lines >= self.info.height {
            self.clear(fill_color);
            return;
        }

        let stride = self.info.stride as u64;
        let copy_height = self.info.height - lines;
        let copy_bytes = copy_height as u64 * stride;

        let dst = self.info.base;
        let src = self.info.base + (lines as u64 * stride);

        // SAFETY: dst < src and copy_bytes fits in mapped fb.
        unsafe {
            asm_fb::memcpy(dst, src, copy_bytes);
        }

        self.fill_rect(
            0,
            self.info.height - lines,
            self.info.width,
            lines,
            fill_color,
        );
    }
}
