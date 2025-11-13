use crate::tui::renderer::{Screen, EFI_GREEN, EFI_BLACK};

pub struct Panel {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub title: &'static str,
}

impl Panel {
    pub fn new(x: usize, y: usize, width: usize, height: usize, title: &'static str) -> Self {
        Self {
            x,
            y,
            width,
            height,
            title,
        }
    }

    pub fn render(&self, screen: &mut Screen) {
        // Top border with title
        let mut buf = [0u8; 128];
        let mut idx = 0;

        buf[idx] = b'+'; idx += 1;
        buf[idx] = b'-'; idx += 1;
        
        // Title
        for &b in self.title.as_bytes() {
            if idx >= buf.len() { break; }
            buf[idx] = b;
            idx += 1;
        }
        
        buf[idx] = b'-'; idx += 1;
        
        // Fill rest
        while idx < self.width - 1 && idx < buf.len() {
            buf[idx] = b'-';
            idx += 1;
        }
        buf[idx] = b'+'; idx += 1;

        let top = core::str::from_utf8(&buf[..idx]).unwrap_or("");
        screen.put_str_at(self.x, self.y, top, EFI_GREEN, EFI_BLACK);

        // Side borders
        for i in 1..self.height - 1 {
            screen.put_char_at(self.x, self.y + i, '|', EFI_GREEN, EFI_BLACK);
            screen.put_char_at(self.x + self.width - 1, self.y + i, '|', EFI_GREEN, EFI_BLACK);
        }

        // Bottom border
        idx = 0;
        buf[idx] = b'+'; idx += 1;
        for _ in 1..self.width - 1 {
            if idx >= buf.len() { break; }
            buf[idx] = b'-';
            idx += 1;
        }
        buf[idx] = b'+'; idx += 1;

        let bottom = core::str::from_utf8(&buf[..idx]).unwrap_or("");
        screen.put_str_at(self.x, self.y + self.height - 1, bottom, EFI_GREEN, EFI_BLACK);
    }
}
