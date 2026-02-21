use crate::canvas::Canvas;
use crate::color::Color;
use crate::draw::shapes::{rect_fill, rect_outline};
use crate::event::{Event, EventResult};
use crate::theme::Theme;
use super::Widget;

pub struct Panel {
    bg: Option<Color>,
    border: bool,
    width: u32,
    height: u32,
}

impl Panel {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            bg: None,
            border: true,
            width,
            height,
        }
    }

    pub fn with_bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    pub fn with_border(mut self, border: bool) -> Self {
        self.border = border;
        self
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }
}

impl Widget for Panel {
    fn size_hint(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let h = canvas.height();
        let bg = self.bg.unwrap_or(theme.bg);

        rect_fill(canvas, 0, 0, w, h, bg);

        if self.border {
            rect_outline(canvas, 0, 0, w, h, 1, theme.border);
        }
    }

    fn handle_event(&mut self, _event: &Event) -> EventResult {
        EventResult::Ignored
    }
}
