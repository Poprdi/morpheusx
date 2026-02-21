use crate::color::{Color, PixelFormat};
use crate::rect::Rect;

pub trait Canvas {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn stride(&self) -> u32;
    fn pixel_format(&self) -> PixelFormat;

    fn put_pixel(&mut self, x: u32, y: u32, color: Color);
    fn get_pixel(&self, x: u32, y: u32) -> Color;
    fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color);
    fn blit(&mut self, dst_x: u32, dst_y: u32, src: &[u32], src_w: u32, src_h: u32);
    fn blit_blend(
        &mut self,
        dst_x: u32,
        dst_y: u32,
        src: &[u32],
        src_w: u32,
        src_h: u32,
        format: PixelFormat,
    );

    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width(), self.height())
    }

    fn clear(&mut self, color: Color) {
        let w = self.width();
        let h = self.height();
        self.fill_rect(0, 0, w, h, color);
    }
}
