//! Screen — framebuffer text console with the same API as the old UEFI version.
//! Under the hood: display crate's TextConsole rendering 8x16 glyphs via ASM.

use alloc::vec;
use alloc::vec::Vec;
use morpheus_display::console::TextConsole;
use morpheus_display::framebuffer::Framebuffer;
use morpheus_display::types::FramebufferInfo;

// VGA/EFI text color indices — same values the display crate expects
pub const EFI_BLACK: usize = 0x00;
pub const EFI_BLUE: usize = 0x01;
pub const EFI_DARKGREEN: usize = 0x02;
pub const EFI_GREEN: usize = 0x02;
pub const EFI_CYAN: usize = 0x03;
pub const EFI_RED: usize = 0x04;
pub const EFI_MAGENTA: usize = 0x05;
pub const EFI_BROWN: usize = 0x06;
pub const EFI_LIGHTGRAY: usize = 0x07;
pub const EFI_DARKGRAY: usize = 0x08;
pub const EFI_LIGHTBLUE: usize = 0x09;
pub const EFI_LIGHTGREEN: usize = 0x0A;
pub const EFI_LIGHTCYAN: usize = 0x0B;
pub const EFI_LIGHTRED: usize = 0x0C;
pub const EFI_LIGHTMAGENTA: usize = 0x0D;
pub const EFI_YELLOW: usize = 0x0E;
pub const EFI_WHITE: usize = 0x0F;

pub struct Screen {
    console: TextConsole,
    width: usize,
    height: usize,
    pub mask: Vec<Vec<bool>>,
}

impl Screen {
    /// Build from raw framebuffer info gathered pre-EBS. Heap must be alive.
    pub unsafe fn from_framebuffer(info: FramebufferInfo) -> Self {
        let fb = Framebuffer::new(info);
        let console = TextConsole::new(fb);
        let width = console.cols();
        let height = console.rows();

        let mut mask = Vec::with_capacity(height);
        for _ in 0..height {
            mask.push(vec![false; width]);
        }

        Self {
            console,
            width,
            height,
            mask,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }

    pub fn center_x(&self, content_width: usize) -> usize {
        if self.width > content_width {
            (self.width - content_width) / 2
        } else {
            0
        }
    }

    pub fn center_y(&self, content_height: usize) -> usize {
        if self.height > content_height {
            (self.height - content_height) / 2
        } else {
            0
        }
    }

    pub fn center_xy(&self, content_width: usize, content_height: usize) -> (usize, usize) {
        (self.center_x(content_width), self.center_y(content_height))
    }

    pub fn clear(&mut self) {
        self.console.clear();
        for row in &mut self.mask {
            for cell in row {
                *cell = false;
            }
        }
    }

    pub fn set_color(&mut self, fg: usize, bg: usize) {
        let attr = (fg & 0x0F) | ((bg & 0x0F) << 4);
        self.console.set_attribute(attr as u8);
    }

    pub fn set_cursor(&mut self, x: usize, y: usize) {
        self.console.set_cursor(x, y);
    }

    pub fn put_char(&mut self, ch: char) {
        self.console.write_char(ch);
    }

    pub fn put_str(&mut self, s: &str) {
        self.console.write_str(s);
    }

    pub fn put_char_at(&mut self, x: usize, y: usize, ch: char, fg: usize, bg: usize) {
        self.set_cursor(x, y);
        self.set_color(fg, bg);
        self.console.write_char(ch);
    }

    pub fn put_str_at(&mut self, x: usize, y: usize, s: &str, fg: usize, bg: usize) {
        self.set_cursor(x, y);
        self.set_color(fg, bg);
        self.console.write_str(s);

        if y < self.height {
            for (i, _) in s.chars().enumerate() {
                let pos = x + i;
                if pos < self.width {
                    self.mask[y][pos] = true;
                }
            }
        }
    }

    pub fn draw_block(&mut self, lines: &[&str]) {
        for line in lines {
            self.console.write_str(line);
            self.console.write_str("\r\n");
        }
    }

    pub fn draw_block_colored(&mut self, lines: &[&str], fg: usize, bg: usize) {
        self.set_color(fg, bg);
        self.draw_block(lines);
        self.set_color(EFI_WHITE, EFI_BLACK);
    }

    pub fn draw_centered_block(
        &mut self,
        lines: &[&str],
        width: usize,
        start_y: usize,
        fg: usize,
        bg: usize,
    ) {
        let x_offset = if self.width > width {
            (self.width - width) / 2
        } else {
            0
        };
        for (i, line) in lines.iter().enumerate() {
            let y = start_y + i;
            if y < self.height {
                self.put_str_at(x_offset, y, line, fg, bg);
            }
        }
    }

    #[inline]
    pub fn set_colors(&mut self, fg: usize, bg: usize) {
        self.set_color(fg, bg);
    }
    #[inline]
    pub fn print(&mut self, s: &str) {
        self.put_str(s);
    }
    #[inline]
    pub fn print_char(&mut self, ch: char) {
        self.put_char(ch);
    }
}
