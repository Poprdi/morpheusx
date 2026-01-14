//! Helper types and constants for the Distro Downloader UI.

use morpheus_core::iso::MAX_ISOS;

// Layout constants
pub const HEADER_Y: usize = 0;
pub const CATEGORY_Y: usize = 3;
pub const LIST_Y: usize = 5;
pub const DETAILS_Y: usize = 14;
pub const FOOTER_Y: usize = 19;
pub const VISIBLE_ITEMS: usize = 8;

/// Action returned from UI input handling
#[derive(Debug, Clone, Copy)]
pub enum ManageAction {
    /// Continue UI loop
    Continue,
    /// Exit UI
    Exit,
}

/// Helper: pad or truncate string to exact length
pub fn pad_or_truncate(s: &str, len: usize) -> alloc::string::String {
    use alloc::string::String;
    let mut result = String::with_capacity(len);
    for (i, c) in s.chars().enumerate() {
        if i >= len {
            break;
        }
        result.push(c);
    }
    while result.len() < len {
        result.push(' ');
    }
    result
}

/// Format size in MB with right-aligned padding
pub fn format_size_mb(mb: u64) -> alloc::string::String {
    use alloc::string::String;
    use core::fmt::Write;
    let mut s = String::with_capacity(12);
    let _ = write!(s, "{:>8}", mb);
    s
}
