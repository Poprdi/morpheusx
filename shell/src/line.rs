extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use libmorpheus::io;
use libmorpheus::process;

use crate::console::Console;
use crate::fb::Framebuffer;

const MAX_LINE: usize = 1024;
const MAX_HISTORY: usize = 128;

pub struct LineEditor {
    history: Vec<String>,
    buf: [u8; MAX_LINE],
    len: usize,
}

impl LineEditor {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            buf: [0u8; MAX_LINE],
            len: 0,
        }
    }

    pub fn read_line(&mut self, interrupted: &dyn Fn() -> bool) -> Option<String> {
        self.len = 0;
        let mut byte = [0u8; 1];

        loop {
            if interrupted() {
                return None;
            }

            let n = io::read_stdin(&mut byte);
            if n == 0 {
                process::yield_cpu();
                continue;
            }

            if interrupted() {
                return None;
            }

            match byte[0] {
                b'\r' | b'\n' => {
                    io::print("\n");
                    let line = self.current_str();
                    self.push_history(&line);
                    return Some(line);
                }
                0x08 | 0x7F => {
                    if self.len > 0 {
                        self.len -= 1;
                        io::print("\x08 \x08");
                    }
                }
                0x03 => return None, // Ctrl+C
                0x0C => {
                    // Ctrl+L: clear and reprint
                    io::print("\x1b[2J\x1b[H");
                    return Some(String::from("\x0c"));
                }
                0x15 => {
                    // Ctrl+U: kill line
                    while self.len > 0 {
                        self.len -= 1;
                        io::print("\x08 \x08");
                    }
                }
                0x17 => {
                    // Ctrl+W: kill word
                    while self.len > 0 && self.buf[self.len - 1] == b' ' {
                        self.len -= 1;
                        io::print("\x08 \x08");
                    }
                    while self.len > 0 && self.buf[self.len - 1] != b' ' {
                        self.len -= 1;
                        io::print("\x08 \x08");
                    }
                }
                c if (0x20..0x7F).contains(&c) => {
                    if self.len < MAX_LINE {
                        self.buf[self.len] = c;
                        self.len += 1;
                        let s = &self.buf[self.len - 1..self.len];
                        // Safety: single ASCII byte is valid UTF-8
                        io::print(unsafe { core::str::from_utf8_unchecked(s) });
                    }
                }
                _ => {}
            }
        }
    }

    fn current_str(&self) -> String {
        let s = core::str::from_utf8(&self.buf[..self.len]).unwrap_or("");
        String::from(s)
    }

    fn push_history(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(last) = self.history.last() {
            if last == trimmed {
                return;
            }
        }
        if self.history.len() >= MAX_HISTORY {
            self.history.remove(0);
        }
        self.history.push(String::from(trimmed));
    }
}

/// Read a line using the framebuffer console.
pub fn read_line_fb(
    fb: &Framebuffer,
    con: &mut Console,
    prompt_col: u32,
    interrupted: &dyn Fn() -> bool,
) -> Option<String> {
    let mut buf = [0u8; MAX_LINE];
    let mut len: usize = 0;
    let mut byte = [0u8; 1];

    loop {
        if interrupted() {
            return None;
        }

        let n = io::read_stdin(&mut byte);
        if n == 0 {
            process::yield_cpu();
            continue;
        }

        if interrupted() {
            return None;
        }

        match byte[0] {
            b'\r' | b'\n' => {
                let s = core::str::from_utf8(&buf[..len]).unwrap_or("");
                return Some(String::from(s));
            }
            0x08 | 0x7F => {
                if len > 0 {
                    len -= 1;
                    con.backspace(fb);
                }
            }
            0x03 => return None,
            0x0C => {
                con.clear(fb);
                return Some(String::from("\x0c"));
            }
            0x15 => {
                len = 0;
                con.kill_to_start(fb, prompt_col);
            }
            0x17 => {
                // Ctrl+W: kill word
                while len > 0 && buf[len - 1] == b' ' {
                    len -= 1;
                    con.backspace(fb);
                }
                while len > 0 && buf[len - 1] != b' ' {
                    len -= 1;
                    con.backspace(fb);
                }
            }
            c if (0x20..0x7F).contains(&c) => {
                if len < MAX_LINE {
                    buf[len] = c;
                    len += 1;
                    con.write_char(fb, c as char);
                }
            }
            _ => {}
        }
    }
}
