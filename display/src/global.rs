//! Global console state and print! macros.
//!
//! This module provides a global static console that can be initialized
//! once and used throughout the system via print!/println! macros.

use core::fmt::{self, Write};

/// Global writer that implements core::fmt::Write.
/// Can be initialized with either backend.
pub struct GlobalWriter;

impl Write for GlobalWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // TODO: Forward to active backend
        // For now, this is a stub
        let _ = s;
        Ok(())
    }
}

/// Print without newline.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::global::GlobalWriter, $($arg)*);
    }};
}

/// Print with newline.
#[macro_export]
macro_rules! println {
    () => {{ $crate::print!("\n"); }};
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n");
    }};
}
