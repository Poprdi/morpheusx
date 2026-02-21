use alloc::string::String;
use crate::canvas::Canvas;
use crate::draw::glyph::draw_string;
use crate::draw::shapes::rect_outline;
use crate::event::{Event, EventResult, Key, KeyEvent};
use crate::font;
use crate::theme::Theme;
use super::Widget;

pub struct TextInput {
    text: String,
    cursor: usize,
    focused: bool,
    max_len: usize,
    scroll_offset: usize,
}

impl TextInput {
    pub fn new(max_len: usize) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            focused: false,
            max_len,
            scroll_offset: 0,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
        self.cursor = self.text.len();
        self.scroll_offset = 0;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    pub fn take_text(&mut self) -> String {
        let t = core::mem::take(&mut self.text);
        self.cursor = 0;
        self.scroll_offset = 0;
        t
    }

    fn visible_cols(&self, canvas_w: u32) -> usize {
        let inner_w = canvas_w.saturating_sub(2) as usize;
        inner_w / font::FONT_WIDTH as usize
    }

    fn update_scroll(&mut self, vis_cols: usize) {
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + vis_cols {
            self.scroll_offset = self.cursor.saturating_sub(vis_cols) + 1;
        }
    }
}

impl Widget for TextInput {
    fn size_hint(&self) -> (u32, u32) {
        let w = (self.max_len as u32) * font::FONT_WIDTH + 4;
        (w, font::FONT_HEIGHT + 4)
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let h = canvas.height();

        canvas.clear(theme.input_bg);
        rect_outline(canvas, 0, 0, w, h, 1, if self.focused { theme.accent } else { theme.border });

        let vis_cols = self.visible_cols(w);
        let visible_text: &str = if self.text.len() > self.scroll_offset {
            let end = (self.scroll_offset + vis_cols).min(self.text.len());
            &self.text[self.scroll_offset..end]
        } else {
            ""
        };

        draw_string(canvas, 2, 2, visible_text, theme.input_fg, theme.input_bg, &font::FONT_DATA);

        if self.focused {
            let cursor_x = (self.cursor.saturating_sub(self.scroll_offset)) as u32 * font::FONT_WIDTH + 2;
            let cursor_y = 2u32;
            if cursor_x < w.saturating_sub(2) {
                canvas.fill_rect(cursor_x, cursor_y, 1, font::FONT_HEIGHT, theme.input_cursor);
            }
        }
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        let Event::KeyPress(KeyEvent { key, modifiers }) = event else {
            return EventResult::Ignored;
        };

        match key {
            Key::Char(c) => {
                if self.text.len() < self.max_len && !modifiers.ctrl {
                    if self.cursor >= self.text.len() {
                        self.text.push(*c);
                    } else {
                        self.text.insert(self.cursor, *c);
                    }
                    self.cursor += 1;
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.text.remove(self.cursor);
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            Key::Delete => {
                if self.cursor < self.text.len() {
                    self.text.remove(self.cursor);
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            Key::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                EventResult::Consumed
            }
            Key::Right => {
                if self.cursor < self.text.len() {
                    self.cursor += 1;
                }
                EventResult::Consumed
            }
            Key::Home => {
                self.cursor = 0;
                EventResult::Consumed
            }
            Key::End => {
                self.cursor = self.text.len();
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }
}
