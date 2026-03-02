use crate::buffer::OffscreenBuffer;
use crate::canvas::Canvas;
use crate::color::PixelFormat;
use crate::draw::glyph::draw_string;
use crate::draw::shapes::{hline, rect_fill};
use crate::font;
use crate::rect::Rect;
use crate::theme::Theme;
use alloc::string::String;

pub const TITLE_BAR_HEIGHT: u32 = 20;
pub const BORDER_WIDTH: u32 = 1;

pub struct Window {
    pub id: u32,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub buffer: OffscreenBuffer,
    pub dirty: bool,
    pub visible: bool,
    pub focused: bool,
    pub decorations: bool,
    pub alpha: u8,
}

impl Window {
    pub fn new(
        id: u32,
        title: &str,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        format: PixelFormat,
    ) -> Self {
        Self {
            id,
            title: String::from(title),
            x,
            y,
            width,
            height,
            buffer: OffscreenBuffer::new(width, height, format),
            dirty: true,
            visible: true,
            focused: false,
            decorations: true,
            alpha: 255,
        }
    }

    pub fn content_rect(&self) -> Rect {
        Rect::new(
            self.x.max(0) as u32,
            self.y.max(0) as u32,
            self.width,
            self.height,
        )
    }

    pub fn outer_rect(&self) -> Rect {
        if self.decorations {
            let ox = (self.x - BORDER_WIDTH as i32).max(0) as u32;
            let oy = (self.y - TITLE_BAR_HEIGHT as i32 - BORDER_WIDTH as i32).max(0) as u32;
            let ow = self.width + BORDER_WIDTH * 2;
            let oh = self.height + TITLE_BAR_HEIGHT + BORDER_WIDTH * 2;
            Rect::new(ox, oy, ow, oh)
        } else {
            self.content_rect()
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn render_decorations(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        if !self.decorations {
            return;
        }

        let outer = self.outer_rect();
        let bounds = canvas.bounds();
        if outer.intersect(bounds).is_none() {
            return;
        }

        let border_color = if self.focused {
            theme.accent
        } else {
            theme.border
        };
        let title_bg = if self.focused {
            theme.title_bg
        } else {
            theme.bg
        };
        let title_fg = if self.focused {
            theme.title_fg
        } else {
            theme.fg
        };

        let ox = outer.x;
        let oy = outer.y;
        let ow = outer.w;
        let tb_h = TITLE_BAR_HEIGHT;

        // Title bar background
        rect_fill(canvas, ox, oy, ow, tb_h, title_bg);

        // Title text (left-aligned with padding)
        let text_y = oy + (tb_h.saturating_sub(font::FONT_HEIGHT)) / 2;
        draw_string(
            canvas,
            ox + 4,
            text_y,
            &self.title,
            title_fg,
            title_bg,
            &font::FONT_DATA,
        );

        // Close button [X] right-aligned
        let close_x = ox + ow.saturating_sub(4 * font::FONT_WIDTH + 2);
        draw_string(
            canvas,
            close_x,
            text_y,
            "[X]",
            title_fg,
            title_bg,
            &font::FONT_DATA,
        );

        // Bottom border of title bar
        hline(canvas, ox, oy + tb_h.saturating_sub(1), ow, border_color);

        // Left border
        let content_bottom = oy + tb_h + self.height + BORDER_WIDTH;
        canvas.fill_rect(
            ox,
            oy + tb_h,
            BORDER_WIDTH,
            self.height + BORDER_WIDTH,
            border_color,
        );

        // Right border
        let right_x = ox + ow.saturating_sub(BORDER_WIDTH);
        canvas.fill_rect(
            right_x,
            oy + tb_h,
            BORDER_WIDTH,
            self.height + BORDER_WIDTH,
            border_color,
        );

        // Bottom border
        hline(
            canvas,
            ox,
            content_bottom.saturating_sub(1),
            ow,
            border_color,
        );
    }

    pub fn content_origin(&self) -> (u32, u32) {
        (self.x.max(0) as u32, self.y.max(0) as u32)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.buffer.resize(width, height);
        self.dirty = true;
    }
}
