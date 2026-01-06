//! String handling utilities
//!
//! ISO9660 uses various string encodings: ASCII, d-characters, a-characters.

/// Trim trailing spaces from byte slice
pub fn trim_trailing_spaces(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b' ' {
        end -= 1;
    }
    &bytes[..end]
}

/// Convert ISO9660 d-characters to string
///
/// d-characters: A-Z, 0-9, _
pub fn dchars_to_str(bytes: &[u8]) -> Result<&str, core::str::Utf8Error> {
    let trimmed = trim_trailing_spaces(bytes);
    core::str::from_utf8(trimmed)
}

/// Convert ISO9660 a-characters to string
///
/// a-characters: A-Z, 0-9, space, !, ", %, &, ', (, ), *, +, ,, -, ., /, :, ;, <, =, >, ?
pub fn achars_to_str(bytes: &[u8]) -> Result<&str, core::str::Utf8Error> {
    let trimmed = trim_trailing_spaces(bytes);
    core::str::from_utf8(trimmed)
}

/// Validate filename against ISO9660 Level 1 rules
///
/// Level 1: 8.3 format, uppercase A-Z 0-9 _
pub fn is_valid_level1_filename(name: &str) -> bool {
    // TODO: Validate format
    !name.is_empty()
}

/// Strip version suffix from filename (e.g., "FILE.TXT;1" -> "FILE.TXT")
/// Also removes trailing dot if present (e.g., "FILE.;1" -> "FILE")
pub fn strip_version(name: &str) -> &str {
    let base = name.split(';').next().unwrap_or(name);
    if let Some(stripped) = base.strip_suffix('.') {
        stripped
    } else {
        base
    }
}
