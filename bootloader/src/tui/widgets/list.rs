use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN};

pub struct ListItem {
    pub label: &'static str,
}

pub struct List {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub items: [ListItem; 10],
    pub item_count: usize,
    pub selected_index: usize,
    pub scroll_offset: usize,
}

impl List {
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
            items: [
                ListItem { label: "" }, ListItem { label: "" },
                ListItem { label: "" }, ListItem { label: "" },
                ListItem { label: "" }, ListItem { label: "" },
                ListItem { label: "" }, ListItem { label: "" },
                ListItem { label: "" }, ListItem { label: "" },
            ],
            item_count: 0,
            selected_index: 0,
            scroll_offset: 0,
        }
    }

    pub fn add_item(&mut self, label: &'static str) {
        if self.item_count < self.items.len() {
            self.items[self.item_count].label = label;
            self.item_count += 1;
        }
    }

    pub fn select_next(&mut self) {
        if self.selected_index < self.item_count - 1 {
            self.selected_index += 1;
            if self.selected_index >= self.scroll_offset + self.height {
                self.scroll_offset += 1;
            }
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            if self.selected_index < self.scroll_offset {
                self.scroll_offset -= 1;
            }
        }
    }

    pub fn render(&self, screen: &mut Screen) {
        let visible_end = (self.scroll_offset + self.height).min(self.item_count);
        
        for i in self.scroll_offset..visible_end {
            let y = self.y + (i - self.scroll_offset);
            let is_selected = i == self.selected_index;
            
            let (fg, bg) = if is_selected {
                (EFI_BLACK, EFI_LIGHTGREEN)
            } else {
                (EFI_GREEN, EFI_BLACK)
            };

            // Build line
            let mut buf = [0u8; 128];
            let mut idx = 0;

            if is_selected {
                buf[idx] = b'>'; idx += 1;
                buf[idx] = b' '; idx += 1;
            } else {
                buf[idx] = b' '; idx += 1;
                buf[idx] = b' '; idx += 1;
            }

            for &b in self.items[i].label.as_bytes() {
                if idx >= buf.len() { break; }
                buf[idx] = b;
                idx += 1;
            }

            let text = core::str::from_utf8(&buf[..idx]).unwrap_or("");
            screen.put_str_at(self.x, y, text, fg, bg);
        }
    }
}
