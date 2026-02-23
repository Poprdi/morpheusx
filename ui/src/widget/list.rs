use super::Widget;
use crate::canvas::Canvas;
use crate::draw::glyph::draw_string;
use crate::draw::shapes::rect_fill;
use crate::event::{Event, EventResult, Key, KeyEvent};
use crate::font;
use crate::theme::Theme;
use alloc::string::String;
use alloc::vec::Vec;

pub struct List {
    items: Vec<String>,
    selected: usize,
    scroll_top: usize,
    focused: bool,
}

impl List {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            scroll_top: 0,
            focused: false,
        }
    }

    pub fn with_items(items: Vec<String>) -> Self {
        Self {
            items,
            selected: 0,
            scroll_top: 0,
            focused: false,
        }
    }

    pub fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
        self.selected = 0;
        self.scroll_top = 0;
    }

    pub fn push(&mut self, item: String) {
        self.items.push(item);
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&str> {
        self.items.get(self.selected).map(|s| s.as_str())
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.selected = 0;
        self.scroll_top = 0;
    }

    fn visible_rows(&self, h: u32) -> usize {
        (h / font::FONT_HEIGHT) as usize
    }

    fn ensure_visible(&mut self, vis: usize) {
        if vis == 0 {
            return;
        }
        if self.selected < self.scroll_top {
            self.scroll_top = self.selected;
        } else if self.selected >= self.scroll_top + vis {
            self.scroll_top = self.selected.saturating_sub(vis) + 1;
        }
    }
}

impl Widget for List {
    fn size_hint(&self) -> (u32, u32) {
        let max_w = self.items.iter().map(|s| s.len()).max().unwrap_or(20) as u32;
        let rows = self.items.len().min(10) as u32;
        (max_w * font::FONT_WIDTH + 4, rows * font::FONT_HEIGHT)
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let h = canvas.height();
        canvas.clear(theme.bg);

        let vis = self.visible_rows(h);
        let text_cols = w / font::FONT_WIDTH;

        for i in 0..vis {
            let idx = self.scroll_top + i;
            if idx >= self.items.len() {
                break;
            }

            let y = i as u32 * font::FONT_HEIGHT;
            let is_selected = idx == self.selected;

            let (fg, bg) = if is_selected && self.focused {
                (theme.selection_fg, theme.selection_bg)
            } else if is_selected {
                (theme.fg, theme.border)
            } else {
                (theme.fg, theme.bg)
            };

            rect_fill(canvas, 0, y, w, font::FONT_HEIGHT, bg);

            let item = &self.items[idx];
            let display: &str = if item.len() > text_cols as usize {
                &item[..text_cols as usize]
            } else {
                item
            };
            draw_string(canvas, 2, y, display, fg, bg, &font::FONT_DATA);
        }
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        if self.items.is_empty() {
            return EventResult::Ignored;
        }

        let Event::KeyPress(KeyEvent { key, .. }) = event else {
            return EventResult::Ignored;
        };

        match key {
            Key::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                    self.ensure_visible(10);
                }
                EventResult::Consumed
            }
            Key::Down => {
                if self.selected + 1 < self.items.len() {
                    self.selected += 1;
                    self.ensure_visible(10);
                }
                EventResult::Consumed
            }
            Key::Home => {
                self.selected = 0;
                self.scroll_top = 0;
                EventResult::Consumed
            }
            Key::End => {
                self.selected = self.items.len().saturating_sub(1);
                self.ensure_visible(10);
                EventResult::Consumed
            }
            Key::PageUp => {
                self.selected = self.selected.saturating_sub(10);
                self.ensure_visible(10);
                EventResult::Consumed
            }
            Key::PageDown => {
                self.selected = (self.selected + 10).min(self.items.len().saturating_sub(1));
                self.ensure_visible(10);
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
