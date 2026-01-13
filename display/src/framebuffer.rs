//! Raw framebuffer access via ASM primitives.
//!
//! All hardware-facing operations go through standalone ASM functions.
//! This provides compiler-proof deterministic memory access.

use crate::asm::fb as asm_fb;
use crate::types::{Color, FramebufferInfo, PixelFormat};

/// Raw framebuffer access using ASM primitives.
pub struct Framebuffer {
    info: FramebufferInfo,
}

impl Framebuffer {
    /// Create a new framebuffer accessor.
    ///
    /// # Safety
    /// The caller must ensure `info.base` points to valid framebuffer memory
    /// that remains mapped for the lifetime of this struct.
    pub const unsafe fn new(info: FramebufferInfo) -> Self {
        Self { info }
    }

    /// Get framebuffer info.
    pub fn info(&self) -> &FramebufferInfo {
        &self.info
    }

    /// Get width in pixels.
    pub fn width(&self) -> u32 {
        self.info.width
    }

    /// Get height in pixels.
    pub fn height(&self) -> u32 {
        self.info.height
    }

    /// Convert color to pixel value based on format.
    #[inline]
    fn color_to_pixel(&self, color: Color) -> u32 {
        match self.info.format {
            PixelFormat::Bgrx => color.to_bgrx(),
            PixelFormat::Rgbx => color.to_rgbx(),
            _ => color.to_bgrx(), // Fallback
        }
    }

    /// Calculate pixel address.
    #[inline]
    fn pixel_addr(&self, x: u32, y: u32) -> u64 {
        // CRITICAL: stride is in BYTES, not pixels
        self.info.base + (y as u64 * self.info.stride as u64) + (x as u64 * 4)
    }

    /// Put a single pixel at (x, y) using ASM.
    ///
    /// Does nothing if coordinates are out of bounds.
    #[inline]
    pub fn put_pixel(&mut self, x: u32, y: u32, color: Color) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }

        let pixel_value = self.color_to_pixel(color);
        let addr = self.pixel_addr(x, y);

        // ALL framebuffer writes go through ASM
        unsafe {
            asm_fb::write32(addr, pixel_value);
        }
    }

    /// Fill a rectangle with a solid color using ASM memset.
    ///
    /// Optimized: uses asm_fb_memset32 for full rows.
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        let x_end = (x + w).min(self.info.width);
        let y_end = (y + h).min(self.info.height);
        let actual_w = x_end.saturating_sub(x);

        if actual_w == 0 {
            return;
        }

        let pixel_value = self.color_to_pixel(color);

        // Fill row by row using ASM memset32
        for py in y..y_end {
            let row_addr = self.pixel_addr(x, py);
            unsafe {
                asm_fb::memset32(row_addr, pixel_value, actual_w as u64);
            }
        }
    }

    /// Clear the entire screen with a color.
    pub fn clear(&mut self, color: Color) {
        let pixel_value = self.color_to_pixel(color);
        let stride = self.info.stride as u64;

        // Clear row by row (handles stride padding correctly)
        for y in 0..self.info.height {
            let row_addr = self.info.base + (y as u64 * stride);
            unsafe {
                asm_fb::memset32(row_addr, pixel_value, self.info.width as u64);
            }
        }
    }

    /// Scroll the screen up by `lines` pixel rows using ASM memcpy.
    ///
    /// Fills the bottom with `fill_color`.
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

        // Copy rows up using ASM memcpy
        unsafe {
            asm_fb::memcpy(dst, src, copy_bytes);
        }

        // Fill the bottom with fill_color
        self.fill_rect(
            0,
            self.info.height - lines,
            self.info.width,
            lines,
            fill_color,
        );
    }
}
