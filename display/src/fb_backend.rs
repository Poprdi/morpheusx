//! Framebuffer-based TextOutput implementation.
//!
//! This is the post-ExitBootServices backend that renders directly to the framebuffer.

use crate::console::TextConsole;
use crate::framebuffer::Framebuffer;
use crate::types::FramebufferInfo;
use crate::TextOutput;

/// Framebuffer-based text output.
pub struct FbTextOutput {
    console: TextConsole,
}

impl FbTextOutput {
    /// Create a new framebuffer text output.
    ///
    /// # Safety
    /// The framebuffer info must point to valid framebuffer memory
    /// that remains mapped for the lifetime of this struct.
    pub unsafe fn new(info: FramebufferInfo) -> Self {
        let fb = Framebuffer::new(info);
        let console = TextConsole::new(fb);
        Self { console }
    }

    /// Get mutable access to the underlying console.
    pub fn console_mut(&mut self) -> &mut TextConsole {
        &mut self.console
    }
}

impl TextOutput for FbTextOutput {
    fn reset(&mut self) {
        self.console.set_attribute(0x07); // Light gray on black
        self.console.clear();
    }

    fn clear(&mut self) {
        self.console.clear();
    }

    fn set_cursor(&mut self, col: usize, row: usize) {
        self.console.set_cursor(col, row);
    }

    fn set_attribute(&mut self, attr: u8) {
        self.console.set_attribute(attr);
    }

    fn write_char(&mut self, c: char) {
        self.console.write_char(c);
    }

    fn write_str(&mut self, s: &str) {
        self.console.write_str(s);
    }

    fn cols(&self) -> usize {
        self.console.cols()
    }

    fn rows(&self) -> usize {
        self.console.rows()
    }
}
