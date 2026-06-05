//! Leveled process logging. Each call emits exactly ONE line in a single
//! `SYS_WRITE` (via [`crate::io`]'s buffered writer), so logs never interleave
//! with other cores on the serial console. Lines are auto-tagged with the
//! calling crate's name and the level is ANSI-colored (the kernel's FB console
//! and a serial terminal both render it). Output goes to stderr (fd 2), keeping
//! it out of a program's stdout data stream.
//!
//! ```ignore
//! libmorpheus::info!("spawned compd pid={}", pid);   // [INFO] init: spawned compd pid=3
//! libmorpheus::warn!("retrying ({}/{})", n, max);
//! libmorpheus::error!("failed: {:#x}", err);
//! ```

use crate::io::FdWriter;
use core::fmt::Write as _;

const RESET: &str = "\x1b[0m";

#[derive(Clone, Copy)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    /// (label, ANSI color).
    #[inline]
    fn parts(self) -> (&'static str, &'static str) {
        match self {
            Level::Error => ("ERROR", "\x1b[31m"),
            Level::Warn => ("WARN", "\x1b[33m"),
            Level::Info => ("INFO", "\x1b[32m"),
            Level::Debug => ("DEBUG", "\x1b[90m"),
        }
    }
}

/// Format `<color>[LEVEL]<reset> tag: <msg>\n` into one buffered writer so the
/// whole line is a single atomic `SYS_WRITE`.
#[doc(hidden)]
pub fn _log(level: Level, tag: &str, args: core::fmt::Arguments<'_>) {
    let (label, color) = level.parts();
    let mut w = FdWriter::new(2);
    let _ = w.write_str(color);
    let _ = w.write_str("[");
    let _ = w.write_str(label);
    let _ = w.write_str("]");
    let _ = w.write_str(RESET);
    let _ = w.write_str(" ");
    let _ = w.write_str(tag);
    let _ = w.write_str(": ");
    let _ = w.write_fmt(args);
    let _ = w.write_str("\n");
    // FdWriter flushes the whole line on drop.
}

/// `env!("CARGO_PKG_NAME")` expands in the *caller's* crate, so each process is
/// auto-tagged with its own name with no per-process setup.
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::log::_log($crate::log::Level::Error, env!("CARGO_PKG_NAME"), format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::log::_log($crate::log::Level::Warn, env!("CARGO_PKG_NAME"), format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::log::_log($crate::log::Level::Info, env!("CARGO_PKG_NAME"), format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        $crate::log::_log($crate::log::Level::Debug, env!("CARGO_PKG_NAME"), format_args!($($arg)*))
    };
}
