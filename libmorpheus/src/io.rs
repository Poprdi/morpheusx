//! Console I/O — read from stdin, write to stdout.

use crate::raw::*;

/// Write a string to the kernel console (serial port, fd 1 = stdout).
pub fn print(s: &str) {
    if s.is_empty() {
        return;
    }
    unsafe {
        syscall3(SYS_WRITE, 1, s.as_ptr() as u64, s.len() as u64);
    }
}

/// Write a string followed by a newline.
pub fn println(s: &str) {
    print(s);
    print("\n");
}

/// Write to a specific fd.
pub fn write_fd(fd: u32, data: &[u8]) -> Result<usize, u64> {
    if data.is_empty() {
        return Ok(0);
    }
    let ret = unsafe { syscall3(SYS_WRITE, fd as u64, data.as_ptr() as u64, data.len() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Read from a specific fd.
pub fn read_fd(fd: u32, buf: &mut [u8]) -> Result<usize, u64> {
    if buf.is_empty() {
        return Ok(0);
    }
    let ret = unsafe { syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Blocking read from stdin (fd 0).
///
/// Returns the number of bytes read.  Blocks until at least one byte
/// is available (keyboard input).
pub fn read_stdin(buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let ret = unsafe { syscall3(SYS_READ, 0, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if crate::is_error(ret) {
        0
    } else {
        ret as usize
    }
}

/// Read a line from stdin into `buf`, echoing characters to stdout.
///
/// Returns the number of bytes read (excluding the trailing newline).
/// The newline is NOT included in the buffer.
pub fn read_line(buf: &mut [u8]) -> usize {
    let mut pos = 0;
    let mut ch = [0u8; 1];
    loop {
        let n = read_stdin(&mut ch);
        if n == 0 {
            continue;
        }
        match ch[0] {
            b'\r' | b'\n' => {
                print("\n");
                return pos;
            }
            // Backspace (0x08 or 0x7F)
            0x08 | 0x7F => {
                if pos > 0 {
                    pos -= 1;
                    print("\x08 \x08"); // erase character
                }
            }
            c => {
                if pos < buf.len() {
                    buf[pos] = c;
                    pos += 1;
                    // Echo the character.
                    let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_ref(&c)) };
                    print(s);
                }
            }
        }
    }
}
