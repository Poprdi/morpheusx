use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN};

pub struct ProgressBar {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub progress: usize, // 0-100
    pub label: &'static str,
}

impl ProgressBar {
    pub fn new(x: usize, y: usize, width: usize, label: &'static str) -> Self {
        Self {
            x,
            y,
            width,
            progress: 0,
            label,
        }
    }

    pub fn set_progress(&mut self, percent: usize) {
        self.progress = percent.min(100);
    }

    pub fn render(&self, screen: &mut Screen) {
        // Draw label
        screen.put_str_at(self.x, self.y, self.label, EFI_GREEN, EFI_BLACK);
        
        // Draw progress bar
        let bar_width = self.width - 2;
        let filled = (bar_width * self.progress) / 100;
        
        let mut buf = [0u8; 128];
        let mut idx = 0;

        buf[idx] = b'['; idx += 1;
        
        for i in 0..bar_width {
            if i < filled {
                buf[idx] = b'=';
            } else if i == filled && self.progress < 100 {
                buf[idx] = b'>';
            } else {
                buf[idx] = b' ';
            }
            idx += 1;
        }
        
        buf[idx] = b']'; idx += 1;

        let text = core::str::from_utf8(&buf[..idx]).unwrap_or("");
        screen.put_str_at(self.x, self.y + 1, text, EFI_LIGHTGREEN, EFI_BLACK);
    }
}
