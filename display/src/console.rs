//! Text console with cursor tracking and character rendering.

use crate::colors::{attr_bg, attr_fg, efi};
use crate::font::{get_glyph_or_space, FONT_HEIGHT, FONT_WIDTH};
use crate::framebuffer::Framebuffer;
use crate::types::Color;

/// Text console that renders characters to a framebuffer.
pub struct TextConsole {
    fb: Framebuffer,
    /// Current cursor column (0-indexed).
    cursor_col: usize,
    /// Current cursor row (0-indexed).
    cursor_row: usize,
    /// Number of text columns.
    cols: usize,
    /// Number of text rows.
    rows: usize,
    /// Current foreground color.
    fg_color: Color,
    /// Current background color.
    bg_color: Color,
    /// Current EFI attribute.
    attr: u8,
}

impl TextConsole {
    /// Create a new text console from a framebuffer.
    ///
    /// # Safety
    /// The framebuffer must be valid for the lifetime of this console.
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
        }
    }

    /// Get number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Get number of rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Set cursor position.
    pub fn set_cursor(&mut self, col: usize, row: usize) {
        self.cursor_col = col.min(self.cols.saturating_sub(1));
        self.cursor_row = row.min(self.rows.saturating_sub(1));
    }

    /// Set text attribute.
    pub fn set_attribute(&mut self, attr: u8) {
        self.attr = attr;
        self.fg_color = attr_fg(attr);
        self.bg_color = attr_bg(attr);
    }

    /// Clear the screen.
    pub fn clear(&mut self) {
        self.fb.clear(self.bg_color);
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    /// Render a single character at the current cursor position.
    fn render_char(&mut self, c: char) {
        let glyph = get_glyph_or_space(c);
        let px = self.cursor_col * FONT_WIDTH;
        let py = self.cursor_row * FONT_HEIGHT;

        for (row_idx, &row_bits) in glyph.iter().enumerate() {
            for col_idx in 0..FONT_WIDTH {
                let bit = (row_bits >> (7 - col_idx)) & 1;
                let color = if bit == 1 {
                    self.fg_color
                } else {
                    self.bg_color
                };
                self.fb
                    .put_pixel((px + col_idx) as u32, (py + row_idx) as u32, color);
            }
        }
    }

    /// Scroll the console up by one line.
    fn scroll_up_one(&mut self) {
        self.fb.scroll_up(FONT_HEIGHT as u32, self.bg_color);
    }

    /// Advance cursor, handling line wrap and scroll.
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

    /// Handle newline.
    fn newline(&mut self) {
        self.cursor_col = 0;
        self.cursor_row += 1;
        if self.cursor_row >= self.rows {
            self.cursor_row = self.rows - 1;
            self.scroll_up_one();
        }
    }

    /// Write a single character.
    pub fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.cursor_col = 0,
            '\t' => {
                // Tab to next 8-column boundary
                let next_tab = (self.cursor_col + 8) & !7;
                while self.cursor_col < next_tab && self.cursor_col < self.cols {
                    self.render_char(' ');
                    self.advance_cursor();
                }
            }
            c if c >= ' ' && c <= '~' => {
                self.render_char(c);
                self.advance_cursor();
            }
            _ => {
                // Non-printable: render as space
                self.render_char(' ');
                self.advance_cursor();
            }
        }
    }

    /// Write a string.
    pub fn write_str(&mut self, s: &str) {
        for c in s.chars() {
            self.write_char(c);
        }
    }
}
