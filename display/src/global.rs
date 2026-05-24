//! Global writer driving `print!` / `println!`.

use core::fmt::{self, Write};

pub struct GlobalWriter;

impl Write for GlobalWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // TODO: forward to active backend; currently a stub.
        let _ = s;
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::global::GlobalWriter, $($arg)*);
    }};
}

#[macro_export]
macro_rules! println {
    () => {{ $crate::print!("\n"); }};
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n");
    }};
}
