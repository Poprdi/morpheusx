use crate::canvas::Canvas;
use crate::color::{Color, PixelFormat};
use crate::rect::Rect;

pub struct ClipCanvas<'a, C: Canvas> {
    inner: &'a mut C,
    clip: Rect,
}

impl<'a, C: Canvas> ClipCanvas<'a, C> {
    pub fn new(inner: &'a mut C, clip: Rect) -> Self {
        let bounded = clip
            .intersect(inner.bounds())
            .unwrap_or(Rect::zero());
        Self {
            inner,
            clip: bounded,
        }
    }
}

impl<'a, C: Canvas> Canvas for ClipCanvas<'a, C> {
    #[inline]
    fn width(&self) -> u32 {
        self.clip.w
    }

    #[inline]
    fn height(&self) -> u32 {
        self.clip.h
    }

    #[inline]
    fn stride(&self) -> u32 {
        self.inner.stride()
    }

    #[inline]
    fn pixel_format(&self) -> PixelFormat {
        self.inner.pixel_format()
    }

    #[inline]
    fn put_pixel(&mut self, x: u32, y: u32, color: Color) {
        if x >= self.clip.w || y >= self.clip.h {
            return;
        }
        self.inner
            .put_pixel(self.clip.x + x, self.clip.y + y, color);
    }

    #[inline]
    fn get_pixel(&self, x: u32, y: u32) -> Color {
        if x >= self.clip.w || y >= self.clip.h {
            return Color::TRANSPARENT;
        }
        self.inner.get_pixel(self.clip.x + x, self.clip.y + y)
    }

    fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        let local = match Rect::new(x, y, w, h).intersect(self.bounds()) {
            Some(c) => c,
            None => return,
        };
        self.inner.fill_rect(
            self.clip.x + local.x,
            self.clip.y + local.y,
            local.w,
            local.h,
            color,
        );
    }

    fn blit(&mut self, dst_x: u32, dst_y: u32, src: &[u32], src_w: u32, src_h: u32) {
        let local = match Rect::new(dst_x, dst_y, src_w, src_h).intersect(self.bounds()) {
            Some(c) => c,
            None => return,
        };
        let sx_off = local.x - dst_x;
        let sy_off = local.y - dst_y;

        for row in 0..local.h {
            let src_start = ((sy_off + row) * src_w + sx_off) as usize;
            let src_end = src_start + local.w as usize;
            if src_end > src.len() {
                break;
            }
            for col in 0..local.w {
                let si = src_start + col as usize;
                self.inner.put_pixel(
                    self.clip.x + local.x + col,
                    self.clip.y + local.y + row,
                    Color::from_packed(src[si], self.inner.pixel_format()),
                );
            }
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
        let local = match Rect::new(dst_x, dst_y, src_w, src_h).intersect(self.bounds()) {
            Some(c) => c,
            None => return,
        };
        let sx_off = local.x - dst_x;
        let sy_off = local.y - dst_y;

        for row in 0..local.h {
            for col in 0..local.w {
                let si = ((sy_off + row) * src_w + sx_off + col) as usize;
                if si >= src.len() {
                    continue;
                }
                let src_color = Color::from_packed(src[si], format);
                if src_color.a == 0 {
                    continue;
                }
                let dx = self.clip.x + local.x + col;
                let dy = self.clip.y + local.y + row;
                if src_color.a == 255 {
                    self.inner.put_pixel(dx, dy, src_color);
                } else {
                    let dst_color = self.inner.get_pixel(dx, dy);
                    self.inner.put_pixel(dx, dy, src_color.blend_over(dst_color));
                }
            }
        }
    }
}
