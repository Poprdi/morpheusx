//! UEFI SimpleTextOutput passthrough backend.
//!
//! This is the pre-ExitBootServices backend that forwards all calls
//! to the UEFI firmware's SimpleTextOutput protocol.

use crate::TextOutput;

/// UEFI Simple Text Output Protocol interface.
///
/// This matches the EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL structure.
#[repr(C)]
pub struct SimpleTextOutputProtocol {
    pub reset: unsafe extern "efiapi" fn(*mut Self, bool) -> usize,
    pub output_string: unsafe extern "efiapi" fn(*mut Self, *const u16) -> usize,
    pub test_string: unsafe extern "efiapi" fn(*mut Self, *const u16) -> usize,
    pub query_mode: unsafe extern "efiapi" fn(*mut Self, usize, *mut usize, *mut usize) -> usize,
    pub set_mode: unsafe extern "efiapi" fn(*mut Self, usize) -> usize,
    pub set_attribute: unsafe extern "efiapi" fn(*mut Self, usize) -> usize,
    pub clear_screen: unsafe extern "efiapi" fn(*mut Self) -> usize,
    pub set_cursor_position: unsafe extern "efiapi" fn(*mut Self, usize, usize) -> usize,
    pub enable_cursor: unsafe extern "efiapi" fn(*mut Self, bool) -> usize,
    pub mode: *mut SimpleTextOutputMode,
}

/// UEFI Simple Text Output Mode.
#[repr(C)]
pub struct SimpleTextOutputMode {
    pub max_mode: i32,
    pub mode: i32,
    pub attribute: i32,
    pub cursor_column: i32,
    pub cursor_row: i32,
    pub cursor_visible: bool,
}

/// UEFI-based text output that passes through to firmware.
pub struct UefiTextOutput {
    protocol: *mut SimpleTextOutputProtocol,
    cols: usize,
    rows: usize,
}

impl UefiTextOutput {
    /// Create a new UEFI text output from a protocol pointer.
    ///
    /// # Safety
    /// The protocol pointer must be valid and remain valid for the lifetime
    /// of this struct. Must only be used before ExitBootServices.
    pub unsafe fn new(protocol: *mut SimpleTextOutputProtocol) -> Self {
        let mut cols: usize = 80;
        let mut rows: usize = 25;

        // Query current mode dimensions
        let mode = (*protocol).mode;
        if !mode.is_null() {
            let current_mode = (*mode).mode as usize;
            let _ = ((*protocol).query_mode)(protocol, current_mode, &mut cols, &mut rows);
        }

        Self {
            protocol,
            cols,
            rows,
        }
    }

    /// Convert a Rust &str to a UCS-2 buffer on the stack.
    /// Returns the buffer and its length (including null terminator).
    fn str_to_ucs2<const N: usize>(s: &str) -> [u16; N] {
        let mut buf = [0u16; N];
        for (i, c) in s.chars().take(N - 1).enumerate() {
            buf[i] = if c as u32 <= 0xFFFF {
                c as u16
            } else {
                '?' as u16
            };
        }
        buf
    }
}

impl TextOutput for UefiTextOutput {
    fn reset(&mut self) {
        unsafe {
            ((*self.protocol).reset)(self.protocol, false);
        }
    }

    fn clear(&mut self) {
        unsafe {
            ((*self.protocol).clear_screen)(self.protocol);
        }
    }

    fn set_cursor(&mut self, col: usize, row: usize) {
        unsafe {
            ((*self.protocol).set_cursor_position)(self.protocol, col, row);
        }
    }

    fn set_attribute(&mut self, attr: u8) {
        unsafe {
            ((*self.protocol).set_attribute)(self.protocol, attr as usize);
        }
    }

    fn write_char(&mut self, c: char) {
        let buf: [u16; 2] = [c as u16, 0];
        unsafe {
            ((*self.protocol).output_string)(self.protocol, buf.as_ptr());
        }
    }

    fn write_str(&mut self, s: &str) {
        // Process in chunks to avoid large stack allocations
        const CHUNK_SIZE: usize = 128;
        let mut buf = [0u16; CHUNK_SIZE];
        let mut idx = 0;

        for c in s.chars() {
            if c == '\n' {
                // UEFI needs \r\n for newlines
                if idx > 0 {
                    buf[idx] = 0;
                    unsafe {
                        ((*self.protocol).output_string)(self.protocol, buf.as_ptr());
                    }
                    idx = 0;
                }
                let crlf: [u16; 3] = ['\r' as u16, '\n' as u16, 0];
                unsafe {
                    ((*self.protocol).output_string)(self.protocol, crlf.as_ptr());
                }
            } else {
                buf[idx] = if c as u32 <= 0xFFFF {
                    c as u16
                } else {
                    '?' as u16
                };
                idx += 1;
                if idx >= CHUNK_SIZE - 1 {
                    buf[idx] = 0;
                    unsafe {
                        ((*self.protocol).output_string)(self.protocol, buf.as_ptr());
                    }
                    idx = 0;
                }
            }
        }

        if idx > 0 {
            buf[idx] = 0;
            unsafe {
                ((*self.protocol).output_string)(self.protocol, buf.as_ptr());
            }
        }
    }

    fn cols(&self) -> usize {
        self.cols
    }

    fn rows(&self) -> usize {
        self.rows
    }

    fn enable_cursor(&mut self, visible: bool) {
        unsafe {
            ((*self.protocol).enable_cursor)(self.protocol, visible);
        }
    }
}
