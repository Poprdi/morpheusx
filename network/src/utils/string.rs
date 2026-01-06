//! String conversion utilities for UEFI environments.
//!
//! Provides no_std compatible string operations:
//! - ASCII/UTF-16 conversion (required for UEFI APIs)
//! - Hex parsing for HTTP headers and debugging
//! - Case conversion without std
//!
//! # Examples
//!
//! ```ignore
//! use morpheus_network::utils::string::{ascii_to_utf16, utf16_to_ascii};
//!
//! let utf16 = ascii_to_utf16("Hello");
//! let ascii = utf16_to_ascii(&utf16).unwrap();
//! assert_eq!(ascii, "Hello");
//! ```

use alloc::string::String;
use alloc::vec::Vec;

/// Convert ASCII string to UTF-16 (null-terminated).
///
/// UEFI uses UTF-16 (UCS-2) for most string operations.
/// This function converts ASCII bytes to UTF-16 code units.
///
/// # Arguments
///
/// * `ascii` - ASCII string to convert
///
/// # Returns
///
/// Vec of UTF-16 code units (null-terminated)
///
/// # Panics
///
/// Does not panic; non-ASCII bytes are passed through as-is.
pub fn ascii_to_utf16(ascii: &str) -> Vec<u16> {
    let mut result: Vec<u16> = ascii.bytes().map(|b| b as u16).collect();
    result.push(0); // Null terminator
    result
}

/// Convert ASCII string to UTF-16 without null terminator.
///
/// Useful when building larger UTF-16 buffers.
pub fn ascii_to_utf16_no_null(ascii: &str) -> Vec<u16> {
    ascii.bytes().map(|b| b as u16).collect()
}

/// Convert UTF-16 to ASCII string.
///
/// # Arguments
///
/// * `utf16` - UTF-16 code units (may or may not be null-terminated)
///
/// # Returns
///
/// ASCII string if all characters are ASCII, None otherwise.
pub fn utf16_to_ascii(utf16: &[u16]) -> Option<String> {
    let mut result = String::with_capacity(utf16.len());
    
    for &code_unit in utf16 {
        // Stop at null terminator
        if code_unit == 0 {
            break;
        }
        
        // Check if ASCII (0-127)
        if code_unit > 127 {
            return None;
        }
        
        result.push(code_unit as u8 as char);
    }
    
    Some(result)
}

/// Convert UTF-16 to ASCII, replacing non-ASCII with '?'.
///
/// More lenient than `utf16_to_ascii` - never fails.
pub fn utf16_to_ascii_lossy(utf16: &[u16]) -> String {
    let mut result = String::with_capacity(utf16.len());
    
    for &code_unit in utf16 {
        if code_unit == 0 {
            break;
        }
        
        if code_unit <= 127 {
            result.push(code_unit as u8 as char);
        } else {
            result.push('?');
        }
    }
    
    result
}

/// Parse a hexadecimal string to usize.
///
/// Used for parsing chunk sizes in chunked transfer encoding.
///
/// # Arguments
///
/// * `hex` - Hex string (without 0x prefix)
///
/// # Returns
///
/// Parsed value, or None if invalid.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(parse_hex("1a"), Some(26));
/// assert_eq!(parse_hex("FF"), Some(255));
/// assert_eq!(parse_hex("invalid"), None);
/// ```
pub fn parse_hex(hex: &str) -> Option<usize> {
    if hex.is_empty() {
        return None;
    }
    
    let mut result: usize = 0;
    
    for byte in hex.bytes() {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        
        // Check for overflow
        result = result.checked_mul(16)?;
        result = result.checked_add(digit as usize)?;
    }
    
    Some(result)
}

/// Parse a decimal string to usize.
///
/// Used for parsing Content-Length headers.
///
/// # Arguments
///
/// * `decimal` - Decimal string
///
/// # Returns
///
/// Parsed value, or None if invalid.
pub fn parse_decimal(decimal: &str) -> Option<usize> {
    if decimal.is_empty() {
        return None;
    }
    
    let mut result: usize = 0;
    
    for byte in decimal.bytes() {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            _ => return None,
        };
        
        result = result.checked_mul(10)?;
        result = result.checked_add(digit as usize)?;
    }
    
    Some(result)
}

/// Convert string to lowercase (ASCII only).
///
/// no_std compatible lowercase conversion.
pub fn to_lowercase(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                (c as u8 + 32) as char
            } else {
                c
            }
        })
        .collect()
}

/// Convert string to uppercase (ASCII only).
pub fn to_uppercase(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_lowercase() {
                (c as u8 - 32) as char
            } else {
                c
            }
        })
        .collect()
}

/// Case-insensitive ASCII string comparison.
pub fn eq_ignore_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    
    a.bytes()
        .zip(b.bytes())
        .all(|(a, b)| a.eq_ignore_ascii_case(&b))
}

/// Trim ASCII whitespace from both ends.
pub fn trim_ascii(s: &str) -> &str {
    let bytes = s.as_bytes();
    
    // Find start (skip leading whitespace)
    let start = bytes
        .iter()
        .position(|&b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    
    // Find end (skip trailing whitespace)
    let end = bytes
        .iter()
        .rposition(|&b| !b.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(start);
    
    // SAFETY: We're slicing at ASCII boundaries
    &s[start..end]
}

/// Format a usize as hexadecimal string.
pub fn to_hex(value: usize) -> String {
    if value == 0 {
        return String::from("0");
    }
    
    let mut result = String::new();
    let mut n = value;
    
    while n > 0 {
        let digit = (n % 16) as u8;
        let c = if digit < 10 {
            (b'0' + digit) as char
        } else {
            (b'a' + digit - 10) as char
        };
        result.insert(0, c);
        n /= 16;
    }
    
    result
}

/// URL-encode a string.
///
/// Encodes non-alphanumeric characters (except -_.~) as %XX.
pub fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => {
                result.push('+');
            }
            _ => {
                result.push('%');
                result.push_str(&to_hex_byte(byte));
            }
        }
    }
    
    result
}

/// URL-decode a string.
///
/// Decodes %XX sequences and + to space.
pub fn url_decode(s: &str) -> Option<String> {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hex = core::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                let byte = parse_hex(hex)? as u8;
                result.push(byte as char);
                i += 3;
            }
            b'+' => {
                result.push(' ');
                i += 1;
            }
            b => {
                result.push(b as char);
                i += 1;
            }
        }
    }
    
    Some(result)
}

/// Format a byte as two hex characters.
fn to_hex_byte(byte: u8) -> String {
    let high = byte >> 4;
    let low = byte & 0x0F;
    
    let c_high = if high < 10 {
        (b'0' + high) as char
    } else {
        (b'A' + high - 10) as char
    };
    
    let c_low = if low < 10 {
        (b'0' + low) as char
    } else {
        (b'A' + low - 10) as char
    };
    
    let mut s = String::with_capacity(2);
    s.push(c_high);
    s.push(c_low);
    s
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // ==================== UTF-16 Conversion Tests ====================

    #[test]
    fn test_ascii_to_utf16_basic() {
        let result = ascii_to_utf16("Hello");
        assert_eq!(result, vec![72, 101, 108, 108, 111, 0]);
    }

    #[test]
    fn test_ascii_to_utf16_empty() {
        let result = ascii_to_utf16("");
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn test_ascii_to_utf16_no_null() {
        let result = ascii_to_utf16_no_null("Hi");
        assert_eq!(result, vec![72, 105]);
    }

    #[test]
    fn test_utf16_to_ascii_basic() {
        let utf16 = vec![72, 101, 108, 108, 111, 0];
        assert_eq!(utf16_to_ascii(&utf16), Some(String::from("Hello")));
    }

    #[test]
    fn test_utf16_to_ascii_no_null() {
        let utf16 = vec![72, 105];
        assert_eq!(utf16_to_ascii(&utf16), Some(String::from("Hi")));
    }

    #[test]
    fn test_utf16_to_ascii_stops_at_null() {
        let utf16 = vec![72, 105, 0, 88, 89];
        assert_eq!(utf16_to_ascii(&utf16), Some(String::from("Hi")));
    }

    #[test]
    fn test_utf16_to_ascii_non_ascii_fails() {
        let utf16 = vec![72, 256, 105]; // 256 is not ASCII
        assert_eq!(utf16_to_ascii(&utf16), None);
    }

    #[test]
    fn test_utf16_to_ascii_lossy() {
        let utf16 = vec![72, 256, 105, 0];
        assert_eq!(utf16_to_ascii_lossy(&utf16), "H?i");
    }

    #[test]
    fn test_utf16_roundtrip() {
        let original = "GET /path HTTP/1.1";
        let utf16 = ascii_to_utf16(original);
        let back = utf16_to_ascii(&utf16).unwrap();
        assert_eq!(back, original);
    }

    // ==================== Hex Parsing Tests ====================

    #[test]
    fn test_parse_hex_basic() {
        assert_eq!(parse_hex("0"), Some(0));
        assert_eq!(parse_hex("1"), Some(1));
        assert_eq!(parse_hex("a"), Some(10));
        assert_eq!(parse_hex("f"), Some(15));
        assert_eq!(parse_hex("10"), Some(16));
        assert_eq!(parse_hex("ff"), Some(255));
        assert_eq!(parse_hex("FF"), Some(255));
        assert_eq!(parse_hex("1a"), Some(26));
        assert_eq!(parse_hex("1A"), Some(26));
    }

    #[test]
    fn test_parse_hex_large() {
        assert_eq!(parse_hex("100"), Some(256));
        assert_eq!(parse_hex("1000"), Some(4096));
        assert_eq!(parse_hex("ffff"), Some(65535));
    }

    #[test]
    fn test_parse_hex_chunk_sizes() {
        // Common HTTP chunk sizes
        assert_eq!(parse_hex("5"), Some(5));
        assert_eq!(parse_hex("1f4"), Some(500));
        assert_eq!(parse_hex("400"), Some(1024));
    }

    #[test]
    fn test_parse_hex_empty() {
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn test_parse_hex_invalid() {
        assert_eq!(parse_hex("g"), None);
        assert_eq!(parse_hex("xyz"), None);
        assert_eq!(parse_hex("12g"), None);
        assert_eq!(parse_hex(" "), None);
        assert_eq!(parse_hex("0x10"), None); // 'x' is invalid
    }

    // ==================== Decimal Parsing Tests ====================

    #[test]
    fn test_parse_decimal_basic() {
        assert_eq!(parse_decimal("0"), Some(0));
        assert_eq!(parse_decimal("1"), Some(1));
        assert_eq!(parse_decimal("123"), Some(123));
        assert_eq!(parse_decimal("1000000"), Some(1000000));
    }

    #[test]
    fn test_parse_decimal_content_length() {
        // Typical Content-Length values
        assert_eq!(parse_decimal("512"), Some(512));
        assert_eq!(parse_decimal("4096"), Some(4096));
        assert_eq!(parse_decimal("1048576"), Some(1048576)); // 1MB
    }

    #[test]
    fn test_parse_decimal_empty() {
        assert_eq!(parse_decimal(""), None);
    }

    #[test]
    fn test_parse_decimal_invalid() {
        assert_eq!(parse_decimal("abc"), None);
        assert_eq!(parse_decimal("12a"), None);
        assert_eq!(parse_decimal("-1"), None);
        assert_eq!(parse_decimal("1.5"), None);
    }

    // ==================== Case Conversion Tests ====================

    #[test]
    fn test_to_lowercase() {
        assert_eq!(to_lowercase("HELLO"), "hello");
        assert_eq!(to_lowercase("Hello World"), "hello world");
        assert_eq!(to_lowercase("already lowercase"), "already lowercase");
        assert_eq!(to_lowercase("MiXeD"), "mixed");
        assert_eq!(to_lowercase(""), "");
    }

    #[test]
    fn test_to_uppercase() {
        assert_eq!(to_uppercase("hello"), "HELLO");
        assert_eq!(to_uppercase("Hello World"), "HELLO WORLD");
        assert_eq!(to_uppercase("ALREADY UPPERCASE"), "ALREADY UPPERCASE");
        assert_eq!(to_uppercase(""), "");
    }

    #[test]
    fn test_eq_ignore_case() {
        assert!(eq_ignore_case("hello", "HELLO"));
        assert!(eq_ignore_case("Hello", "hELLO"));
        assert!(eq_ignore_case("Content-Type", "content-type"));
        assert!(eq_ignore_case("", ""));
        assert!(!eq_ignore_case("hello", "world"));
        assert!(!eq_ignore_case("hello", "helloo"));
    }

    // ==================== Trim Tests ====================

    #[test]
    fn test_trim_ascii_basic() {
        assert_eq!(trim_ascii("  hello  "), "hello");
        assert_eq!(trim_ascii("\t\nhello\r\n"), "hello");
        assert_eq!(trim_ascii("hello"), "hello");
        assert_eq!(trim_ascii(""), "");
        assert_eq!(trim_ascii("   "), "");
    }

    #[test]
    fn test_trim_ascii_preserves_inner() {
        assert_eq!(trim_ascii("  hello world  "), "hello world");
    }

    // ==================== Hex Formatting Tests ====================

    #[test]
    fn test_to_hex() {
        assert_eq!(to_hex(0), "0");
        assert_eq!(to_hex(1), "1");
        assert_eq!(to_hex(10), "a");
        assert_eq!(to_hex(15), "f");
        assert_eq!(to_hex(16), "10");
        assert_eq!(to_hex(255), "ff");
        assert_eq!(to_hex(256), "100");
        assert_eq!(to_hex(4096), "1000");
    }

    #[test]
    fn test_hex_roundtrip() {
        for value in [0, 1, 15, 16, 255, 1000, 65535, 1048576] {
            let hex = to_hex(value);
            let parsed = parse_hex(&hex).unwrap();
            assert_eq!(parsed, value, "Roundtrip failed for {}", value);
        }
    }

    // ==================== URL Encoding Tests ====================

    #[test]
    fn test_url_encode_basic() {
        assert_eq!(url_encode("hello"), "hello");
        assert_eq!(url_encode("Hello World"), "Hello+World");
        assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
    }

    #[test]
    fn test_url_encode_special() {
        assert_eq!(url_encode("test@example.com"), "test%40example.com");
        assert_eq!(url_encode("/path/to/file"), "%2Fpath%2Fto%2Ffile");
    }

    #[test]
    fn test_url_encode_safe_chars() {
        // These should not be encoded
        assert_eq!(url_encode("abc-_.~"), "abc-_.~");
        assert_eq!(url_encode("ABC123"), "ABC123");
    }

    #[test]
    fn test_url_decode_basic() {
        assert_eq!(url_decode("hello"), Some(String::from("hello")));
        assert_eq!(url_decode("Hello+World"), Some(String::from("Hello World")));
        assert_eq!(url_decode("a%3Db"), Some(String::from("a=b")));
    }

    #[test]
    fn test_url_decode_invalid() {
        assert_eq!(url_decode("%"), None);
        assert_eq!(url_decode("%2"), None);
        assert_eq!(url_decode("%GG"), None);
    }

    #[test]
    fn test_url_roundtrip() {
        let original = "Hello World! a=b&c=d";
        let encoded = url_encode(original);
        let decoded = url_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
