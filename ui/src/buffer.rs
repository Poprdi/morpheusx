use alloc::vec;
use alloc::vec::Vec;

use crate::canvas::Canvas;
use crate::color::{Color, PixelFormat};
use crate::rect::Rect;

pub struct OffscreenBuffer {
    pixels: Vec<u32>,
    width: u32,
    height: u32,
    format: PixelFormat,
}

impl OffscreenBuffer {
    pub fn new(width: u32, height: u32, format: PixelFormat) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            pixels: vec![0u32; len],
            width,
            height,
            format,
        }
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.pixels
    }

    pub fn as_mut_slice(&mut self) -> &mut [u32] {
        &mut self.pixels
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        let len = (width as usize).saturating_mul(height as usize);
        self.pixels.resize(len, 0);
        self.pixels.fill(0);
    }
}

impl Canvas for OffscreenBuffer {
    #[inline]
    fn width(&self) -> u32 {
        self.width
    }

    #[inline]
    fn height(&self) -> u32 {
        self.height
    }

    #[inline]
    fn stride(&self) -> u32 {
        self.width
    }

    #[inline]
    fn pixel_format(&self) -> PixelFormat {
        self.format
    }

    #[inline]
    fn put_pixel(&mut self, x: u32, y: u32, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = (y * self.width + x) as usize;
        self.pixels[idx] = color.to_packed(self.format);
    }

    #[inline]
    fn get_pixel(&self, x: u32, y: u32) -> Color {
        if x >= self.width || y >= self.height {
            return Color::TRANSPARENT;
        }
        let idx = (y * self.width + x) as usize;
        Color::from_packed(self.pixels[idx], self.format)
    }

    fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        let clip = match Rect::new(x, y, w, h).intersect(self.bounds()) {
            Some(c) => c,
            None => return,
        };

        let packed = color.to_packed(self.format);

        for row in clip.y..clip.bottom() {
            let start = (row * self.width + clip.x) as usize;
            let end = start + clip.w as usize;
            self.pixels[start..end].fill(packed);
        }
    }

    fn blit(&mut self, dst_x: u32, dst_y: u32, src: &[u32], src_w: u32, src_h: u32) {
        let dst_clip = match Rect::new(dst_x, dst_y, src_w, src_h).intersect(self.bounds()) {
            Some(c) => c,
            None => return,
        };

        let sx_off = dst_clip.x - dst_x;
        let sy_off = dst_clip.y - dst_y;

        for row in 0..dst_clip.h {
            let src_row_start = ((sy_off + row) * src_w + sx_off) as usize;
            let src_row_end = src_row_start + dst_clip.w as usize;
            if src_row_end > src.len() {
                break;
            }

            let dst_row_start = ((dst_clip.y + row) * self.width + dst_clip.x) as usize;
            let dst_row_end = dst_row_start + dst_clip.w as usize;

            self.pixels[dst_row_start..dst_row_end]
                .copy_from_slice(&src[src_row_start..src_row_end]);
        }
    }

    fn blit_blend(
        &mut self,
        dst_x: u32,
        dst_y: u32,
        src: &[u32],
        src_w: u32,
        src_h: u32,
        format: PixelFormat,
    ) {
        let dst_clip = match Rect::new(dst_x, dst_y, src_w, src_h).intersect(self.bounds()) {
            Some(c) => c,
            None => return,
        };

        let sx_off = dst_clip.x - dst_x;
        let sy_off = dst_clip.y - dst_y;

        for row in 0..dst_clip.h {
            for col in 0..dst_clip.w {
                let src_idx = ((sy_off + row) * src_w + sx_off + col) as usize;
                if src_idx >= src.len() {
                    continue;
                }
                let src_color = Color::from_packed(src[src_idx], format);
                if src_color.a == 0 {
                    continue;
                }
                let dx = dst_clip.x + col;
                let dy = dst_clip.y + row;
                if src_color.a == 255 {
                    self.put_pixel(dx, dy, src_color);
                } else {
                    let dst_color = self.get_pixel(dx, dy);
                    self.put_pixel(dx, dy, src_color.blend_over(dst_color));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_zeroed() {
        let buf = OffscreenBuffer::new(10, 10, PixelFormat::Bgrx);
        assert!(buf.as_slice().iter().all(|&p| p == 0));
    }

    #[test]
    fn put_get_pixel() {
        let mut buf = OffscreenBuffer::new(10, 10, PixelFormat::Bgrx);
        let c = Color::rgb(0xAA, 0xBB, 0xCC);
        buf.put_pixel(5, 5, c);
        let got = buf.get_pixel(5, 5);
        assert_eq!(got.r, c.r);
        assert_eq!(got.g, c.g);
        assert_eq!(got.b, c.b);
    }

    #[test]
    fn put_pixel_out_of_bounds_no_panic() {
        let mut buf = OffscreenBuffer::new(10, 10, PixelFormat::Bgrx);
        buf.put_pixel(100, 100, Color::RED);
    }

    #[test]
    fn fill_rect_clips() {
        let mut buf = OffscreenBuffer::new(10, 10, PixelFormat::Bgrx);
        buf.fill_rect(8, 8, 10, 10, Color::WHITE);
        assert_ne!(buf.get_pixel(9, 9), Color::TRANSPARENT);
    }

    #[test]
    fn blit_copies_region() {
        let mut buf = OffscreenBuffer::new(10, 10, PixelFormat::Bgrx);
        let src_data = vec![Color::RED.to_packed(PixelFormat::Bgrx); 9];
        buf.blit(0, 0, &src_data, 3, 3);
        assert_eq!(buf.get_pixel(0, 0).r, 255);
        assert_eq!(buf.get_pixel(2, 2).r, 255);
    }
}
