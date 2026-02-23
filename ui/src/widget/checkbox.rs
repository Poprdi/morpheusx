use super::Widget;
use crate::canvas::Canvas;
use crate::draw::glyph::draw_string;
use crate::event::{Event, EventResult, Key, KeyEvent};
use crate::font;
use crate::theme::Theme;
use alloc::string::String;

pub struct Checkbox {
    label: String,
    checked: bool,
    focused: bool,
}

impl Checkbox {
    pub fn new(label: &str) -> Self {
        Self {
            label: String::from(label),
            checked: false,
            focused: false,
        }
    }

    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn is_checked(&self) -> bool {
        self.checked
    }

    pub fn set_checked(&mut self, checked: bool) {
        self.checked = checked;
    }

    pub fn toggle(&mut self) {
        self.checked = !self.checked;
    }
}

impl Widget for Checkbox {
    fn size_hint(&self) -> (u32, u32) {
        let text_w = (self.label.len() as u32 + 4) * font::FONT_WIDTH;
        (text_w, font::FONT_HEIGHT)
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let fg = if self.focused { theme.accent } else { theme.fg };
        let bg = theme.bg;

        canvas.clear(bg);

        let marker = if self.checked { "[X] " } else { "[ ] " };
        draw_string(canvas, 0, 0, marker, fg, bg, &font::FONT_DATA);
        draw_string(
            canvas,
            4 * font::FONT_WIDTH,
            0,
            &self.label,
            fg,
            bg,
            &font::FONT_DATA,
        );
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        if let Event::KeyPress(KeyEvent { key, .. }) = event {
            match key {
                Key::Enter | Key::Char(' ') => {
                    self.toggle();
                    return EventResult::Consumed;
                }
                _ => {}
            }
        }
        EventResult::Ignored
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }
}
