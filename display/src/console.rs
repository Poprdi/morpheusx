//! 8x16 text console over a framebuffer.

use crate::colors::{attr_bg, attr_fg, efi, make_attr};
use crate::font::{get_glyph_or_space, FONT_HEIGHT, FONT_WIDTH};
use crate::framebuffer::Framebuffer;
use crate::types::Color;

/// Minimal ANSI escape parser state. We only consume CSI SGR (`\x1b[..m`)
/// color sequences so the serial log mirror renders colored, not as literal
/// `[32m` text. Every other escape is swallowed.
#[derive(Clone, Copy)]
enum AnsiState {
    Normal,
    Escape,
    Csi,
}

pub struct TextConsole {
    fb: Framebuffer,
    cursor_col: usize,
    cursor_row: usize,
    cols: usize,
    rows: usize,
    fg_color: Color,
    bg_color: Color,
    attr: u8,
    ansi: AnsiState,
    ansi_param: u16,
}

impl TextConsole {
    /// SAFETY: `fb` must remain valid for the console's lifetime.
    pub unsafe fn new(fb: Framebuffer) -> Self {
        let cols = fb.width() as usize / FONT_WIDTH;
        let rows = fb.height() as usize / FONT_HEIGHT;

        Self {
            fb,
            cursor_col: 0,
            cursor_row: 0,
            cols,
            rows,
            fg_color: attr_fg(efi::DEFAULT_ATTR),
            bg_color: attr_bg(efi::DEFAULT_ATTR),
            attr: efi::DEFAULT_ATTR,
            ansi: AnsiState::Normal,
            ansi_param: 0,
        }
    }

    /// Apply a single SGR parameter. Foreground colors map to the EFI palette;
    /// `0` resets to the default attribute. Unsupported codes are ignored.
    fn apply_sgr(&mut self, code: u16) {
        let fg = match code {
            0 => efi::DEFAULT_ATTR,
            31 => efi::LIGHTRED,
            32 => efi::LIGHTGREEN,
            33 => efi::YELLOW,
            34 => efi::LIGHTBLUE,
            35 => efi::LIGHTMAGENTA,
            36 => efi::LIGHTCYAN,
            37 => efi::WHITE,
            _ => return,
        };
        self.set_attribute(make_attr(fg, efi::BLACK));
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn set_cursor(&mut self, col: usize, row: usize) {
        self.cursor_col = col.min(self.cols.saturating_sub(1));
        self.cursor_row = row.min(self.rows.saturating_sub(1));
    }

    pub fn set_attribute(&mut self, attr: u8) {
        self.attr = attr;
        self.fg_color = attr_fg(attr);
        self.bg_color = attr_bg(attr);
    }

    pub fn clear(&mut self) {
        self.fb.clear(self.bg_color);
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    /// Bg via memset32, then plot fg pixels only.
    fn render_char(&mut self, c: char) {
        let glyph = get_glyph_or_space(c);
        let px = (self.cursor_col * FONT_WIDTH) as u32;
        let py = (self.cursor_row * FONT_HEIGHT) as u32;

        self.fb
            .fill_rect(px, py, FONT_WIDTH as u32, FONT_HEIGHT as u32, self.bg_color);

        for (row_idx, &row_bits) in glyph.iter().enumerate() {
            if row_bits == 0 {
                continue;
            }
            for col_idx in 0..FONT_WIDTH {
                if (row_bits >> (7 - col_idx)) & 1 == 1 {
                    self.fb
                        .put_pixel(px + col_idx as u32, py + row_idx as u32, self.fg_color);
                }
            }
        }
    }

    fn scroll_up_one(&mut self) {
        self.fb.scroll_up(FONT_HEIGHT as u32, self.bg_color);
    }

    fn advance_cursor(&mut self) {
        self.cursor_col += 1;
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= self.rows {
                self.cursor_row = self.rows - 1;
                self.scroll_up_one();
            }
        }
    }

    fn newline(&mut self) {
        self.cursor_col = 0;
        self.cursor_row += 1;
        if self.cursor_row >= self.rows {
            self.cursor_row = self.rows - 1;
            self.scroll_up_one();
        }
    }

    pub fn write_char(&mut self, c: char) {
        // Consume ANSI SGR color escapes before rendering. The live serial
        // mirror feeds bytes one at a time, so the parser state persists
        // across calls.
        match self.ansi {
            AnsiState::Normal => {
                if c == '\x1b' {
                    self.ansi = AnsiState::Escape;
                    return;
                }
            },
            AnsiState::Escape => {
                if c == '[' {
                    self.ansi = AnsiState::Csi;
                    self.ansi_param = 0;
                } else {
                    self.ansi = AnsiState::Normal;
                }
                return;
            },
            AnsiState::Csi => {
                match c {
                    '0'..='9' => {
                        self.ansi_param = self
                            .ansi_param
                            .saturating_mul(10)
                            .saturating_add(c as u16 - '0' as u16);
                    },
                    ';' => {
                        self.apply_sgr(self.ansi_param);
                        self.ansi_param = 0;
                    },
                    'm' => {
                        self.apply_sgr(self.ansi_param);
                        self.ansi = AnsiState::Normal;
                    },
                    _ => self.ansi = AnsiState::Normal,
                }
                return;
            },
        }

        match c {
            '\n' => self.newline(),
            '\r' => self.cursor_col = 0,
            '\x08' => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
                self.render_char(' ');
            },
            '\t' => {
                // Round up to next 8-column tabstop.
                let next_tab = (self.cursor_col + 8) & !7;
                while self.cursor_col < next_tab && self.cursor_col < self.cols {
                    self.render_char(' ');
                    self.advance_cursor();
                }
            },
            c if (' '..='~').contains(&c) => {
                self.render_char(c);
                self.advance_cursor();
            },
            _ => {
                self.render_char(' ');
                self.advance_cursor();
            },
        }
    }

    pub fn write_str(&mut self, s: &str) {
        for c in s.chars() {
            self.write_char(c);
        }
    }
}
