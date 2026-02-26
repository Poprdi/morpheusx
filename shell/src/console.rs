extern crate alloc;

use alloc::string::String;

use crate::fb::Framebuffer;
use crate::font;

const FG: (u8, u8, u8) = (0, 170, 0);
const BG: (u8, u8, u8) = (0, 0, 0);
const CURSOR_COLOR: (u8, u8, u8) = (85, 255, 85);

pub struct Console {
    cols: u32,
    rows: u32,
    cx: u32,
    cy: u32,
    cursor_visible: bool,
}

impl Console {
    pub fn new(fb: &Framebuffer) -> Self {
        let cols = fb.width / font::FONT_WIDTH as u32;
        let rows = fb.height / font::FONT_HEIGHT as u32;
        Self {
            cols,
            rows,
            cx: 0,
            cy: 0,
            cursor_visible: false,
        }
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    pub fn cursor_col(&self) -> u32 {
        self.cx
    }

    pub fn clear(&mut self, fb: &Framebuffer) {
        fb.clear(BG.0, BG.1, BG.2);
        self.cx = 0;
        self.cy = 0;
    }

    pub fn write_str(&mut self, fb: &Framebuffer, s: &str) {
        self.hide_cursor(fb);
        for ch in s.chars() {
            self.put_char(fb, ch);
        }
        self.show_cursor(fb);
    }

    pub fn write_char(&mut self, fb: &Framebuffer, ch: char) {
        self.hide_cursor(fb);
        self.put_char(fb, ch);
        self.show_cursor(fb);
    }

    pub fn backspace(&mut self, fb: &Framebuffer) {
        self.hide_cursor(fb);
        if self.cx > 0 {
            self.cx -= 1;
            self.draw_cell(fb, self.cx, self.cy, ' ', FG, BG);
        }
        self.show_cursor(fb);
    }

    pub fn kill_to_start(&mut self, fb: &Framebuffer, prompt_len: u32) {
        self.hide_cursor(fb);
        let start = prompt_len.min(self.cols);
        for x in start..self.cx {
            self.draw_cell(fb, x, self.cy, ' ', FG, BG);
        }
        self.cx = start;
        self.show_cursor(fb);
    }

    pub fn newline(&mut self, fb: &Framebuffer) {
        self.hide_cursor(fb);
        self.cx = 0;
        self.cy += 1;
        if self.cy >= self.rows {
            self.scroll(fb);
        }
        self.show_cursor(fb);
    }

    fn put_char(&mut self, fb: &Framebuffer, ch: char) {
        match ch {
            '\n' => {
                self.cx = 0;
                self.cy += 1;
                if self.cy >= self.rows {
                    self.scroll(fb);
                }
            }
            '\r' => {
                self.cx = 0;
            }
            '\t' => {
                let next = (self.cx + 4) & !3;
                while self.cx < next && self.cx < self.cols {
                    self.draw_cell(fb, self.cx, self.cy, ' ', FG, BG);
                    self.cx += 1;
                }
                if self.cx >= self.cols {
                    self.cx = 0;
                    self.cy += 1;
                    if self.cy >= self.rows {
                        self.scroll(fb);
                    }
                }
            }
            c if c >= ' ' && (c as u32) < 0x7F => {
                if self.cx >= self.cols {
                    self.cx = 0;
                    self.cy += 1;
                    if self.cy >= self.rows {
                        self.scroll(fb);
                    }
                }
                self.draw_cell(fb, self.cx, self.cy, c, FG, BG);
                self.cx += 1;
            }
            _ => {}
        }
    }

    fn draw_cell(
        &self,
        fb: &Framebuffer,
        col: u32,
        row: u32,
        ch: char,
        fg: (u8, u8, u8),
        bg: (u8, u8, u8),
    ) {
        let glyph = font::get_glyph_or_space(ch);
        let px = col * font::FONT_WIDTH as u32;
        let py = row * font::FONT_HEIGHT as u32;
        fb.draw_glyph(glyph, px, py, fg, bg);
    }

    fn scroll(&mut self, fb: &Framebuffer) {
        fb.scroll_up(font::FONT_HEIGHT as u32, BG.0, BG.1, BG.2);
        self.cy = self.rows.saturating_sub(1);
    }

    fn show_cursor(&mut self, fb: &Framebuffer) {
        if self.cursor_visible {
            return;
        }
        self.cursor_visible = true;
        let glyph = font::get_glyph_or_space('_');
        let px = self.cx * font::FONT_WIDTH as u32;
        let py = self.cy * font::FONT_HEIGHT as u32;
        fb.draw_glyph(glyph, px, py, CURSOR_COLOR, BG);
    }

    fn hide_cursor(&mut self, fb: &Framebuffer) {
        if !self.cursor_visible {
            return;
        }
        self.cursor_visible = false;
        let px = self.cx * font::FONT_WIDTH as u32;
        let py = self.cy * font::FONT_HEIGHT as u32;
        // Erase cursor with blank cell
        fb.draw_glyph(font::get_glyph_or_space(' '), px, py, FG, BG);
    }

    pub fn write_colored(&mut self, fb: &Framebuffer, s: &str, fg: (u8, u8, u8)) {
        self.hide_cursor(fb);
        for ch in s.chars() {
            match ch {
                '\n' => {
                    self.cx = 0;
                    self.cy += 1;
                    if self.cy >= self.rows {
                        self.scroll(fb);
                    }
                }
                c if c >= ' ' && (c as u32) < 0x7F => {
                    if self.cx >= self.cols {
                        self.cx = 0;
                        self.cy += 1;
                        if self.cy >= self.rows {
                            self.scroll(fb);
                        }
                    }
                    self.draw_cell(fb, self.cx, self.cy, c, fg, BG);
                    self.cx += 1;
                }
                _ => {}
            }
        }
        self.show_cursor(fb);
    }

    pub fn render_prompt(&mut self, fb: &Framebuffer, cwd: &str, last_status: i32) {
        self.write_colored(fb, "morpheus", (85, 255, 85));
        self.write_colored(fb, ":", FG);
        self.write_colored(fb, cwd, (85, 85, 255));

        if last_status != 0 {
            let mut buf = [0u8; 16];
            let s = format_i32(last_status, &mut buf);
            self.write_colored(fb, " [", (170, 0, 0));
            self.write_colored(fb, s, (170, 0, 0));
            self.write_colored(fb, "]", (170, 0, 0));
        }

        self.write_colored(fb, "> ", FG);
    }
}

fn format_i32(val: i32, buf: &mut [u8; 16]) -> &str {
    let mut n = val;
    let negative = n < 0;
    if negative {
        n = -n;
    }
    let mut pos = buf.len();
    if n == 0 {
        pos -= 1;
        buf[pos] = b'0';
    } else {
        while n > 0 {
            pos -= 1;
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    if negative {
        pos -= 1;
        buf[pos] = b'-';
    }
    core::str::from_utf8(&buf[pos..]).unwrap_or("?")
}
