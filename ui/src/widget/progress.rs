use super::Widget;
use crate::canvas::Canvas;
use crate::draw::glyph::draw_string;
use crate::draw::shapes::rect_fill;
use crate::event::{Event, EventResult};
use crate::font;
use crate::theme::Theme;

pub struct ProgressBar {
    value: u32,
    max: u32,
    width: u32,
    show_label: bool,
}

impl ProgressBar {
    pub fn new(width: u32) -> Self {
        Self {
            value: 0,
            max: 100,
            width,
            show_label: true,
        }
    }

    pub fn set_value(&mut self, value: u32) {
        self.value = value.min(self.max);
    }

    pub fn set_max(&mut self, max: u32) {
        self.max = max.max(1);
        self.value = self.value.min(self.max);
    }

    pub fn set_show_label(&mut self, show: bool) {
        self.show_label = show;
    }

    pub fn value(&self) -> u32 {
        self.value
    }

    pub fn fraction(&self) -> u32 {
        if self.max == 0 {
            return 0;
        }
        (self.value * 100) / self.max
    }
}

impl Widget for ProgressBar {
    fn size_hint(&self) -> (u32, u32) {
        (self.width, font::FONT_HEIGHT + 4)
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let h = canvas.height();

        rect_fill(canvas, 0, 0, w, h, theme.bg);
        rect_fill(canvas, 0, 0, w, h, theme.border);

        let inner_x = 1u32;
        let inner_y = 1u32;
        let inner_w = w.saturating_sub(2);
        let inner_h = h.saturating_sub(2);

        rect_fill(canvas, inner_x, inner_y, inner_w, inner_h, theme.input_bg);

        let fill_w = if self.max > 0 {
            (inner_w as u64 * self.value as u64 / self.max as u64) as u32
        } else {
            0
        };

        if fill_w > 0 {
            rect_fill(canvas, inner_x, inner_y, fill_w, inner_h, theme.accent);
        }

        if self.show_label {
            let pct = self.fraction();
            let mut buf = [0u8; 8];
            let label = format_pct(pct, &mut buf);
            let label_w = label.len() as u32 * font::FONT_WIDTH;
            let lx = inner_x + inner_w.saturating_sub(label_w) / 2;
            let ly = inner_y + inner_h.saturating_sub(font::FONT_HEIGHT) / 2;

            let fg = if fill_w > lx + label_w / 2 {
                theme.bg
            } else {
                theme.fg
            };
            draw_string(canvas, lx, ly, label, fg, theme.input_bg, &font::FONT_DATA);
        }
    }

    fn handle_event(&mut self, _event: &Event) -> EventResult {
        EventResult::Ignored
    }
}

fn format_pct(pct: u32, buf: &mut [u8; 8]) -> &str {
    let mut n = pct;
    let mut pos = 7;
    buf[pos] = b'%';
    if n == 0 {
        pos -= 1;
        buf[pos] = b'0';
    } else {
        while n > 0 && pos > 0 {
            pos -= 1;
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    if let Ok(s) = core::str::from_utf8(&buf[pos..8]) {
        s
    } else {
        "0%"
    }
}
