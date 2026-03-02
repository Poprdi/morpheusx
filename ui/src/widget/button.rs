use super::Widget;
use crate::canvas::Canvas;
use crate::draw::glyph::draw_string;
use crate::draw::shapes::rect_outline;
use crate::event::{Event, EventResult, Key, KeyEvent};
use crate::font;
use crate::theme::Theme;
use alloc::string::String;

pub struct Button {
    label: String,
    focused: bool,
    pressed: bool,
    on_press: bool,
}

impl Button {
    pub fn new(label: &str) -> Self {
        Self {
            label: String::from(label),
            focused: false,
            pressed: false,
            on_press: false,
        }
    }

    pub fn set_label(&mut self, label: &str) {
        self.label.clear();
        self.label.push_str(label);
    }

    pub fn was_pressed(&mut self) -> bool {
        let p = self.on_press;
        self.on_press = false;
        p
    }
}

impl Widget for Button {
    fn size_hint(&self) -> (u32, u32) {
        let text_w = self.label.len() as u32 * font::FONT_WIDTH;
        (text_w + 4, font::FONT_HEIGHT + 4)
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let bg = if self.focused {
            theme.button_focus_bg
        } else {
            theme.button_bg
        };
        let fg = theme.button_fg;

        canvas.clear(bg);

        let w = canvas.width();
        let h = canvas.height();
        rect_outline(canvas, 0, 0, w, h, 1, theme.border);

        let text_w = self.label.len() as u32 * font::FONT_WIDTH;
        let tx = w.saturating_sub(text_w) / 2;
        let ty = h.saturating_sub(font::FONT_HEIGHT) / 2;
        draw_string(canvas, tx, ty, &self.label, fg, bg, &font::FONT_DATA);
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        if let Event::KeyPress(KeyEvent {
            key: Key::Enter | Key::Char(' '),
            ..
        }) = event
        {
            self.on_press = true;
            self.pressed = true;
            return EventResult::Consumed;
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
