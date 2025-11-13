use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN};

pub struct TextBox {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub buffer: [u8; 64],
    pub length: usize,
    pub selected: bool,
}

impl TextBox {
    pub fn new(x: usize, y: usize, width: usize) -> Self {
        Self {
            x,
            y,
            width,
            buffer: [0u8; 64],
            length: 0,
            selected: false,
        }
    }

    pub fn add_char(&mut self, ch: u8) {
        if self.length < self.buffer.len() && self.length < self.width - 2 {
            self.buffer[self.length] = ch;
            self.length += 1;
        }
    }

    pub fn backspace(&mut self) {
        if self.length > 0 {
            self.length -= 1;
            self.buffer[self.length] = 0;
        }
    }

    pub fn get_text(&self) -> &str {
        core::str::from_utf8(&self.buffer[..self.length]).unwrap_or("")
    }

    pub fn render(&self, screen: &mut Screen) {
        let (fg, bg) = if self.selected {
            (EFI_BLACK, EFI_LIGHTGREEN)
        } else {
            (EFI_GREEN, EFI_BLACK)
        };

        // Draw border
        let mut buf = [0u8; 128];
        let mut idx = 0;

        buf[idx] = b'['; idx += 1;
        
        // Content
        for i in 0..self.length {
            buf[idx] = self.buffer[i];
            idx += 1;
        }
        
        // Padding
        for _ in self.length..(self.width - 2) {
            buf[idx] = b' ';
            idx += 1;
        }
        
        buf[idx] = b']'; idx += 1;

        let text = core::str::from_utf8(&buf[..idx]).unwrap_or("");
        screen.put_str_at(self.x, self.y, text, fg, bg);
    }
}
