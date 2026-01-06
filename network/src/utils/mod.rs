//! Shared utilities for network operations.
//!
//! Provides:
//! - String conversion (UTF-8, UTF-16, ASCII)
//! - Buffer management (RingBuffer, ChunkBuffer)
//! - Hex/decimal parsing
//! - URL encoding/decoding

pub mod string;
pub mod buffer;

pub use string::{
    ascii_to_utf16, ascii_to_utf16_no_null, utf16_to_ascii, utf16_to_ascii_lossy,
    parse_hex, parse_decimal, to_lowercase, to_uppercase, eq_ignore_case,
    trim_ascii, to_hex, url_encode, url_decode,
};

pub use buffer::{
    BufferError, BufferResult, RingBuffer, ChunkBuffer, SliceReader, GrowableBuffer,
};
