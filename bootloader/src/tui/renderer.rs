use crate::SimpleTextOutputProtocol;
use alloc::vec::Vec;

// EFI text colors
pub const EFI_BLACK: usize = 0x00;
pub const EFI_BLUE: usize = 0x01;       // Very dark, good for dim rain
pub const EFI_DARKGREEN: usize = 0x02;  // Dim green for background rain
pub const EFI_GREEN: usize = 0x02;
pub const EFI_CYAN: usize = 0x03;       // Alternative dim color
pub const EFI_LIGHTGREEN: usize = 0x0A;
pub const EFI_WHITE: usize = 0x0F;

fn str_to_ucs2(s: &str, buf: &mut [u16]) {
    let mut i = 0;
    for ch in s.chars() {
        if i >= buf.len() - 1 {
            break;
        }
        buf[i] = ch as u16;
        i += 1;
    }
    buf[i] = 0;
}

pub struct Screen {
    con_out: *mut SimpleTextOutputProtocol,
    width: usize,
    height: usize,
    pub mask: Vec<Vec<bool>>,
}

impl Screen {
    pub fn new(con_out: *mut SimpleTextOutputProtocol) -> Self {
        let (width, height) = Self::get_screen_size(con_out);
        
        // Create dynamic mask based on actual screen size
        let mut mask = Vec::new();
        for _ in 0..height {
            let mut row = Vec::new();
            for _ in 0..width {
                row.push(false);
            }
            mask.push(row);
        }
        
        Self { 
            con_out, 
            width, 
            height,
            mask,
        }
    }

    fn get_screen_size(con_out: *mut SimpleTextOutputProtocol) -> (usize, usize) {
        unsafe {
            let protocol = &mut *con_out;
            
            if protocol.mode.is_null() {
                return (80, 25); // fallback
            }
            
            let mode_info = &*protocol.mode;
            let current_mode = mode_info.mode;
            
            if current_mode < 0 {
                return (80, 25); // fallback
            }
            
            // Query current mode dimensions
            let mut cols: usize = 80;
            let mut rows: usize = 25;
            
            let status = (protocol.query_mode)(
                protocol,
                current_mode as usize,
                &mut cols as *mut usize,
                &mut rows as *mut usize,
            );
            
            // Status 0 = success
            if status == 0 && cols > 0 && rows > 0 {
                (cols, rows)
            } else {
                (80, 25) // fallback
            }
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    // Calculate centered X position for content of given width
    pub fn center_x(&self, content_width: usize) -> usize {
        if self.width > content_width {
            (self.width - content_width) / 2
        } else {
            0
        }
    }

    // Calculate centered Y position for content of given height
    pub fn center_y(&self, content_height: usize) -> usize {
        if self.height > content_height {
            (self.height - content_height) / 2
        } else {
            0
        }
    }

    // Get centered coordinates for content box
    pub fn center_xy(&self, content_width: usize, content_height: usize) -> (usize, usize) {
        (self.center_x(content_width), self.center_y(content_height))
    }

    pub fn clear(&mut self) {
        unsafe {
            let con_out = &mut *self.con_out;
            (con_out.clear_screen)(con_out);
        }
        // Reset mask when clearing
        for row in &mut self.mask {
            for cell in row {
                *cell = false;
            }
        }
    }

    pub fn set_color(&mut self, fg: usize, bg: usize) {
        let attr = fg | (bg << 4);
        unsafe {
            let con_out = &mut *self.con_out;
            (con_out.set_attribute)(con_out, attr);
        }
    }

    pub fn set_cursor(&mut self, x: usize, y: usize) {
        unsafe {
            let con_out = &mut *self.con_out;
            (con_out.set_cursor_position)(con_out, x, y);
        }
    }

    pub fn put_char(&mut self, ch: char) {
        let buf = [ch as u16, 0];
        unsafe {
            let con_out = &mut *self.con_out;
            (con_out.output_string)(con_out, buf.as_ptr());
        }
    }

    pub fn put_str(&mut self, s: &str) {
        let mut buffer = [0u16; 256];
        str_to_ucs2(s, &mut buffer);
        unsafe {
            let con_out = &mut *self.con_out;
            (con_out.output_string)(con_out, buffer.as_ptr());
        }
    }

    pub fn put_char_at(&mut self, x: usize, y: usize, ch: char, fg: usize, bg: usize) {
        self.set_cursor(x, y);
        self.set_color(fg, bg);
        self.put_char(ch);
    }

    pub fn put_str_at(&mut self, x: usize, y: usize, s: &str, fg: usize, bg: usize) {
        self.set_cursor(x, y);
        self.set_color(fg, bg);
        self.put_str(s);
        
        // Update mask for written content
        if y < self.height {
            for (i, _ch) in s.chars().enumerate() {
                let pos = x + i;
                if pos < self.width {
                    self.mask[y][pos] = true;
                }
            }
        }
    }

    // Draw multi-line text block with proper spacing
    pub fn draw_block(&mut self, lines: &[&str]) {
        let mut buffer = [0u16; 512];
        
        for line in lines {
            str_to_ucs2(line, &mut buffer);
            unsafe {
                let con_out = &mut *self.con_out;
                (con_out.output_string)(con_out, buffer.as_ptr());
            }
            
            // Add newline
            str_to_ucs2("\r\n", &mut buffer);
            unsafe {
                let con_out = &mut *self.con_out;
                (con_out.output_string)(con_out, buffer.as_ptr());
            }
        }
    }

    pub fn draw_block_colored(&mut self, lines: &[&str], fg: usize, bg: usize) {
        self.set_color(fg, bg);
        self.draw_block(lines);
        self.set_color(EFI_WHITE, EFI_BLACK);
    }

    // Draw centered text block
    pub fn draw_centered_block(&mut self, lines: &[&str], width: usize, start_y: usize, fg: usize, bg: usize) {
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
}
