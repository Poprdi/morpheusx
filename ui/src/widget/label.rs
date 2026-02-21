use alloc::string::String;
use crate::canvas::Canvas;
use crate::color::Color;
use crate::draw::glyph::draw_string;
use crate::event::{Event, EventResult};
use crate::font;
use crate::theme::Theme;
use super::Widget;

pub struct Label {
    text: String,
    color: Option<Color>,
}

impl Label {
    pub fn new(text: &str) -> Self {
        Self {
            text: String::from(text),
            color: None,
        }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn set_text(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl Widget for Label {
    fn size_hint(&self) -> (u32, u32) {
        let chars = self.text.len() as u32;
        (chars * font::FONT_WIDTH, font::FONT_HEIGHT)
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let fg = self.color.unwrap_or(theme.fg);
        canvas.clear(theme.bg);
        draw_string(canvas, 0, 0, &self.text, fg, theme.bg, &font::FONT_DATA);
    }

    fn handle_event(&mut self, _event: &Event) -> EventResult {
        EventResult::Ignored
    }
}
