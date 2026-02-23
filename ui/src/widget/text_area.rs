use super::Widget;
use crate::canvas::Canvas;
use crate::draw::glyph::draw_string;
use crate::event::{Event, EventResult, Key, KeyEvent};
use crate::font;
use crate::theme::Theme;
use alloc::string::String;
use alloc::vec::Vec;

pub struct TextArea {
    lines: Vec<String>,
    capacity: usize,
    scroll_top: usize,
    focused: bool,
}

impl TextArea {
    pub fn new(line_capacity: usize) -> Self {
        Self {
            lines: Vec::new(),
            capacity: line_capacity,
            scroll_top: 0,
            focused: false,
        }
    }

    pub fn push_line(&mut self, line: &str) {
        if self.lines.len() >= self.capacity {
            self.lines.remove(0);
            if self.scroll_top > 0 {
                self.scroll_top -= 1;
            }
        }
        self.lines.push(String::from(line));
    }

    pub fn push_str(&mut self, text: &str) {
        for line in text.split('\n') {
            self.push_line(line);
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.scroll_top = 0;
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn scroll_to_bottom(&mut self) {
        let vis = self.visible_lines_for_height(u32::MAX);
        if self.lines.len() > vis {
            self.scroll_top = self.lines.len() - vis;
        } else {
            self.scroll_top = 0;
        }
    }

    fn visible_lines_for_height(&self, h: u32) -> usize {
        (h / font::FONT_HEIGHT) as usize
    }

    fn draw_scrollbar(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let h = canvas.height();
        if h == 0 || self.lines.is_empty() {
            return;
        }

        let bar_x = w.saturating_sub(2);
        canvas.fill_rect(bar_x, 0, 2, h, theme.scrollbar_bg);

        let total = self.lines.len() as u32;
        let vis = (h / font::FONT_HEIGHT).max(1);
        if total <= vis {
            return;
        }

        let track_h = h;
        let thumb_h = ((vis as u64 * track_h as u64) / total as u64).max(4) as u32;
        let thumb_y = ((self.scroll_top as u64 * track_h as u64) / total as u64) as u32;
        canvas.fill_rect(
            bar_x,
            thumb_y,
            2,
            thumb_h.min(track_h.saturating_sub(thumb_y)),
            theme.scrollbar_fg,
        );
    }
}

impl Widget for TextArea {
    fn size_hint(&self) -> (u32, u32) {
        (320, 200)
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let h = canvas.height();
        canvas.clear(theme.bg);

        let vis = self.visible_lines_for_height(h);
        let text_w = w.saturating_sub(4) / font::FONT_WIDTH;

        for i in 0..vis {
            let line_idx = self.scroll_top + i;
            if line_idx >= self.lines.len() {
                break;
            }
            let y = i as u32 * font::FONT_HEIGHT;
            let line = &self.lines[line_idx];
            let display: &str = if line.len() > text_w as usize {
                &line[..text_w as usize]
            } else {
                line
            };
            draw_string(canvas, 2, y, display, theme.fg, theme.bg, &font::FONT_DATA);
        }

        self.draw_scrollbar(canvas, theme);
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        let Event::KeyPress(KeyEvent { key, .. }) = event else {
            return EventResult::Ignored;
        };

        match key {
            Key::Up => {
                if self.scroll_top > 0 {
                    self.scroll_top -= 1;
                }
                EventResult::Consumed
            }
            Key::Down => {
                let max = self.lines.len().saturating_sub(1);
                if self.scroll_top < max {
                    self.scroll_top += 1;
                }
                EventResult::Consumed
            }
            Key::PageUp => {
                self.scroll_top = self.scroll_top.saturating_sub(10);
                EventResult::Consumed
            }
            Key::PageDown => {
                let max = self.lines.len().saturating_sub(1);
                self.scroll_top = (self.scroll_top + 10).min(max);
                EventResult::Consumed
            }
            Key::Home => {
                self.scroll_top = 0;
                EventResult::Consumed
            }
            Key::End => {
                self.scroll_to_bottom();
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
