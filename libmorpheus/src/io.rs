//! Console I/O — write to serial/stdout.

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
