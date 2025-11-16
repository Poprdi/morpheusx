use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN};

pub struct Checkbox {
    pub x: usize,
    pub y: usize,
    pub label: &'static str,
    pub checked: bool,
    pub selected: bool,
}

impl Checkbox {
    pub fn new(x: usize, y: usize, label: &'static str) -> Self {
        Self {
            x,
            y,
            label,
            checked: false,
            selected: false,
        }
    }

    pub fn toggle(&mut self) {
        self.checked = !self.checked;
    }

    pub fn render(&self, screen: &mut Screen) {
        let (fg, bg) = if self.selected {
            (EFI_BLACK, EFI_LIGHTGREEN)
        } else {
            (EFI_GREEN, EFI_BLACK)
        };

        // Build checkbox string
        let mut buf = [0u8; 64];
        let mut idx = 0;

        buf[idx] = b'[';
        idx += 1;
        buf[idx] = if self.checked { b'X' } else { b' ' };
        idx += 1;
        buf[idx] = b']';
        idx += 1;
        buf[idx] = b' ';
        idx += 1;

        for &b in self.label.as_bytes() {
            if idx >= buf.len() {
                break;
            }
            buf[idx] = b;
            idx += 1;
        }

        let text = core::str::from_utf8(&buf[..idx]).unwrap_or("");
        screen.put_str_at(self.x, self.y, text, fg, bg);
    }
}
