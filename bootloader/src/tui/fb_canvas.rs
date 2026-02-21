use morpheus_display::asm::fb as asm_fb;
use morpheus_display::types::FramebufferInfo;
use morpheus_ui::canvas::Canvas;
use morpheus_ui::color::{Color, PixelFormat};
use morpheus_ui::rect::Rect;

pub struct FbCanvas {
    base: u64,
    width: u32,
    height: u32,
    stride_bytes: u32,
    format: PixelFormat,
}

impl FbCanvas {
    pub unsafe fn new(info: &FramebufferInfo) -> Self {
        let format = match info.format {
            morpheus_display::types::PixelFormat::Rgbx => PixelFormat::Rgbx,
            _ => PixelFormat::Bgrx,
        };

        Self {
            base: info.base,
            width: info.width,
            height: info.height,
            stride_bytes: info.stride,
            format,
        }
    }

    #[inline]
    fn pixel_addr(&self, x: u32, y: u32) -> u64 {
        self.base + (y as u64 * self.stride_bytes as u64) + (x as u64 * 4)
    }
}

impl Canvas for FbCanvas {
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
        self.stride_bytes / 4
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
        let packed = color.to_packed(self.format);
        let addr = self.pixel_addr(x, y);
        unsafe { asm_fb::write32(addr, packed); }
    }

    #[inline]
    fn get_pixel(&self, x: u32, y: u32) -> Color {
        if x >= self.width || y >= self.height {
            return Color::TRANSPARENT;
        }
        let addr = self.pixel_addr(x, y);
        let packed = unsafe { asm_fb::read32(addr) };
        Color::from_packed(packed, self.format)
    }

    fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        let clip = match Rect::new(x, y, w, h).intersect(self.bounds()) {
            Some(c) => c,
            None => return,
        };

        let packed = color.to_packed(self.format);

        for row in clip.y..clip.bottom() {
            let addr = self.pixel_addr(clip.x, row);
            unsafe { asm_fb::memset32(addr, packed, clip.w as u64); }
        }
    }

    fn blit(&mut self, dst_x: u32, dst_y: u32, src: &[u32], src_w: u32, src_h: u32) {
        let clip = match Rect::new(dst_x, dst_y, src_w, src_h).intersect(self.bounds()) {
            Some(c) => c,
            None => return,
        };

        let sx_off = clip.x - dst_x;
        let sy_off = clip.y - dst_y;

        for row in 0..clip.h {
            let src_start = ((sy_off + row) * src_w + sx_off) as usize;
            let src_end = src_start + clip.w as usize;
            if src_end > src.len() {
                break;
            }

            let dst_addr = self.pixel_addr(clip.x, clip.y + row);
            let src_ptr = src[src_start..src_end].as_ptr() as u64;
            let byte_count = (clip.w as u64) * 4;
            unsafe { asm_fb::memcpy(dst_addr, src_ptr, byte_count); }
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
        let clip = match Rect::new(dst_x, dst_y, src_w, src_h).intersect(self.bounds()) {
            Some(c) => c,
            None => return,
        };

        let sx_off = clip.x - dst_x;
        let sy_off = clip.y - dst_y;

        for row in 0..clip.h {
            for col in 0..clip.w {
                let src_idx = ((sy_off + row) * src_w + sx_off + col) as usize;
                if src_idx >= src.len() {
                    continue;
                }
                let src_color = Color::from_packed(src[src_idx], format);
                if src_color.a == 0 {
                    continue;
                }
                let dx = clip.x + col;
                let dy = clip.y + row;
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
