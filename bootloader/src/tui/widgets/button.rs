use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN};

pub struct Button {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub label: &'static str,
    pub selected: bool,
}

impl Button {
    pub fn new(x: usize, y: usize, label: &'static str) -> Self {
        let width = label.len() + 4; // padding on sides
        Self {
            x,
            y,
            width,
            label,
            selected: false,
        }
    }

    pub fn render(&self, screen: &mut Screen) {
        let (fg, bg) = if self.selected {
            (EFI_BLACK, EFI_LIGHTGREEN) // Inverted when selected
        } else {
            (EFI_GREEN, EFI_BLACK)
        };

        // Draw simple button - just the label with brackets
        let mut buf = [0u8; 64];
        let mut idx = 0;
        
        // Top border
        if self.selected {
            buf[idx] = b'>'; idx += 1;
            buf[idx] = b' '; idx += 1;
        } else {
            buf[idx] = b'['; idx += 1;
            buf[idx] = b' '; idx += 1;
        }
        
        for &b in self.label.as_bytes() {
            buf[idx] = b;
            idx += 1;
        }
        
        buf[idx] = b' '; idx += 1;
        if self.selected {
            buf[idx] = b'<'; idx += 1;
        } else {
            buf[idx] = b']'; idx += 1;
        }
        
        let text = core::str::from_utf8(&buf[..idx]).unwrap_or("");
        screen.put_str_at(self.x, self.y, text, fg, bg);
    }

    pub fn contains_point(&self, x: usize, y: usize) -> bool {
        x >= self.x && x < self.x + self.width &&
        y == self.y
    }
}
