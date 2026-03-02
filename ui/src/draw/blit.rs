use crate::canvas::Canvas;
use crate::color::{Color, PixelFormat};
use crate::rect::Rect;

pub fn blit_opaque(
    canvas: &mut dyn Canvas,
    dst_x: u32,
    dst_y: u32,
    src: &[u32],
    src_w: u32,
    src_h: u32,
) {
    canvas.blit(dst_x, dst_y, src, src_w, src_h);
}

pub fn blit_blend(
    canvas: &mut dyn Canvas,
    dst_x: u32,
    dst_y: u32,
    src: &[u32],
    src_w: u32,
    src_h: u32,
    format: PixelFormat,
) {
    canvas.blit_blend(dst_x, dst_y, src, src_w, src_h, format);
}

pub fn blit_region(
    canvas: &mut dyn Canvas,
    dst_x: u32,
    dst_y: u32,
    src: &[u32],
    src_stride: u32,
    src_rect: Rect,
) {
    let dst_clip = match Rect::new(dst_x, dst_y, src_rect.w, src_rect.h).intersect(canvas.bounds())
    {
        Some(c) => c,
        None => return,
    };

    let sx_off = (dst_clip.x - dst_x) + src_rect.x;
    let sy_off = (dst_clip.y - dst_y) + src_rect.y;
    let format = canvas.pixel_format();

    for row in 0..dst_clip.h {
        for col in 0..dst_clip.w {
            let si = ((sy_off + row) * src_stride + sx_off + col) as usize;
            if si >= src.len() {
                continue;
            }
            canvas.put_pixel(
                dst_clip.x + col,
                dst_clip.y + row,
                Color::from_packed(src[si], format),
            );
        }
    }
}
