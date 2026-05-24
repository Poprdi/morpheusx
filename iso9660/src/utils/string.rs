//! ISO 9660 string decoding (a-/d-characters; see Annex A).

/// Strip trailing space bytes used to pad fixed-width fields.
pub fn trim_trailing_spaces(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b' ' {
        end -= 1;
    }
    &bytes[..end]
}

/// Decode d-characters (A-Z, 0-9, `_`) as UTF-8 after space-padding strip.
pub fn dchars_to_str(bytes: &[u8]) -> Result<&str, core::str::Utf8Error> {
    let trimmed = trim_trailing_spaces(bytes);
    core::str::from_utf8(trimmed)
}

/// Decode a-characters (superset of d-characters plus punctuation).
pub fn achars_to_str(bytes: &[u8]) -> Result<&str, core::str::Utf8Error> {
    let trimmed = trim_trailing_spaces(bytes);
    core::str::from_utf8(trimmed)
}

/// Permissive Level 1 (8.3) validity check; currently rejects only empty names.
pub fn is_valid_level1_filename(name: &str) -> bool {
    !name.is_empty()
}

/// Drop the `;N` version suffix and any trailing dot left behind.
/// `"FILE.TXT;1"` → `"FILE.TXT"`, `"FILE.;1"` → `"FILE"`.
pub fn strip_version(name: &str) -> &str {
    let base = name.split(';').next().unwrap_or(name);
    if let Some(stripped) = base.strip_suffix('.') {
        stripped
    } else {
        base
    }
}
