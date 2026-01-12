//! Chunked transfer encoding decoder.
//!
//! Decodes HTTP chunked transfer encoding as defined in RFC 7230.
//!
//! # Format
//!
//! ```text
//! chunk-size (hex)\r\n
//! chunk-data\r\n
//! ...
//! 0\r\n
//! \r\n
//! ```
//!
//! # Examples
//!
//! ```ignore
//! use morpheus_network::transfer::ChunkedDecoder;
//!
//! let data = b"5\r\nHello\r\n0\r\n\r\n";
//! let result = ChunkedDecoder::decode(data).unwrap();
//! assert_eq!(result, b"Hello");
//! ```

use crate::error::{NetworkError, Result};
use alloc::vec::Vec;

/// State of the chunked decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderState {
    /// Waiting for chunk size line.
    ReadingSize,
    /// Reading chunk data.
    ReadingData,
    /// Expecting \r after chunk data.
    ExpectingCR,
    /// Expecting \n after chunk data.
    ExpectingLF,
    /// Finished reading all chunks.
    Done,
}

/// Chunked transfer encoding decoder.
///
/// Handles incremental decoding for streaming scenarios.
#[derive(Debug)]
pub struct ChunkedDecoder {
    /// Current decoder state.
    state: DecoderState,
    /// Buffer for incomplete chunk size line.
    size_buffer: Vec<u8>,
    /// Expected size of current chunk.
    current_chunk_size: usize,
    /// Bytes read in current chunk.
    chunk_bytes_read: usize,
    /// Decoded output data.
    output: Vec<u8>,
}

impl ChunkedDecoder {
    /// Create a new chunked decoder.
    pub fn new() -> Self {
        Self {
            state: DecoderState::ReadingSize,
            size_buffer: Vec::new(),
            current_chunk_size: 0,
            chunk_bytes_read: 0,
            output: Vec::new(),
        }
    }

    /// Get current decoder state.
    pub fn state(&self) -> DecoderState {
        self.state
    }

    /// Check if decoding is complete.
    pub fn is_done(&self) -> bool {
        self.state == DecoderState::Done
    }

    /// Get decoded output (only valid when done).
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    /// Take ownership of the decoded output.
    pub fn take_output(self) -> Vec<u8> {
        self.output
    }

    /// Decode a complete chunked body in one go.
    ///
    /// Returns the decoded data.
    pub fn decode(data: &[u8]) -> Result<Vec<u8>> {
        let mut decoder = ChunkedDecoder::new();
        decoder.feed(data)?;

        if !decoder.is_done() {
            return Err(NetworkError::InvalidResponse);
        }

        Ok(decoder.take_output())
    }

    /// Feed data to the decoder incrementally.
    ///
    /// Returns the number of bytes consumed.
    pub fn feed(&mut self, data: &[u8]) -> Result<usize> {
        let mut consumed = 0;

        while consumed < data.len() && self.state != DecoderState::Done {
            let byte = data[consumed];
            consumed += 1;

            match self.state {
                DecoderState::ReadingSize => {
                    if byte == b'\n'
                        && !self.size_buffer.is_empty()
                        && self.size_buffer.last() == Some(&b'\r')
                    {
                        // Found end of size line
                        self.size_buffer.pop(); // Remove \r
                        self.parse_chunk_size()?;
                    } else {
                        // Limit size buffer to prevent DoS (max chunk size is ~16 hex chars + extension)
                        if self.size_buffer.len() >= 256 {
                            return Err(NetworkError::InvalidResponse);
                        }
                        self.size_buffer.push(byte);
                    }
                }
                DecoderState::ReadingData => {
                    self.output.push(byte);
                    self.chunk_bytes_read += 1;

                    if self.chunk_bytes_read == self.current_chunk_size {
                        // Finished this chunk, expect trailing CRLF
                        self.state = DecoderState::ExpectingCR;
                    }
                }
                DecoderState::ExpectingCR => {
                    if byte == b'\r' {
                        self.state = DecoderState::ExpectingLF;
                    } else {
                        return Err(NetworkError::InvalidResponse);
                    }
                }
                DecoderState::ExpectingLF => {
                    if byte == b'\n' {
                        // Start reading next chunk size
                        self.state = DecoderState::ReadingSize;
                    } else {
                        return Err(NetworkError::InvalidResponse);
                    }
                }
                DecoderState::Done => break,
            }
        }

        Ok(consumed)
    }

    /// Parse the chunk size from size_buffer.
    fn parse_chunk_size(&mut self) -> Result<()> {
        let size_str =
            core::str::from_utf8(&self.size_buffer).map_err(|_| NetworkError::InvalidResponse)?;

        // Handle chunk extensions (ignore everything after ;)
        let size_part = size_str.split(';').next().unwrap_or("").trim();

        self.current_chunk_size =
            usize::from_str_radix(size_part, 16).map_err(|_| NetworkError::InvalidResponse)?;

        self.size_buffer.clear();
        self.chunk_bytes_read = 0;

        if self.current_chunk_size == 0 {
            // Final chunk
            self.state = DecoderState::Done;
        } else {
            self.state = DecoderState::ReadingData;
        }

        Ok(())
    }

    /// Reset the decoder to initial state.
    pub fn reset(&mut self) {
        self.state = DecoderState::ReadingSize;
        self.size_buffer.clear();
        self.current_chunk_size = 0;
        self.chunk_bytes_read = 0;
        self.output.clear();
    }
}

impl Default for ChunkedDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    // ==================== Basic Decoding ====================

    #[test]
    fn test_decode_single_chunk() {
        let data = b"5\r\nHello\r\n0\r\n\r\n";
        let result = ChunkedDecoder::decode(data).unwrap();
        assert_eq!(result, b"Hello");
    }

    #[test]
    fn test_decode_multiple_chunks() {
        let data = b"5\r\nHello\r\n6\r\n World\r\n0\r\n\r\n";
        let result = ChunkedDecoder::decode(data).unwrap();
        assert_eq!(result, b"Hello World");
    }

    #[test]
    fn test_decode_empty_body() {
        let data = b"0\r\n\r\n";
        let result = ChunkedDecoder::decode(data).unwrap();
        assert!(result.is_empty());
    }

    // ==================== Hex Sizes ====================

    #[test]
    fn test_decode_hex_lowercase() {
        let data = b"a\r\n0123456789\r\n0\r\n\r\n";
        let result = ChunkedDecoder::decode(data).unwrap();
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_decode_hex_uppercase() {
        let data = b"A\r\n0123456789\r\n0\r\n\r\n";
        let result = ChunkedDecoder::decode(data).unwrap();
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_decode_large_chunk_size() {
        // 0x10 = 16 bytes
        let data = b"10\r\n0123456789ABCDEF\r\n0\r\n\r\n";
        let result = ChunkedDecoder::decode(data).unwrap();
        assert_eq!(result, b"0123456789ABCDEF");
    }

    // ==================== Chunk Extensions ====================

    #[test]
    fn test_decode_with_chunk_extension() {
        // Extensions are ignored per RFC 7230
        let data = b"5;name=value\r\nHello\r\n0\r\n\r\n";
        let result = ChunkedDecoder::decode(data).unwrap();
        assert_eq!(result, b"Hello");
    }

    // ==================== Incremental Decoding ====================

    #[test]
    fn test_incremental_feed() {
        let mut decoder = ChunkedDecoder::new();

        // Feed data in parts
        decoder.feed(b"5\r\nHel").unwrap();
        assert!(!decoder.is_done());

        decoder.feed(b"lo\r\n0\r\n\r\n").unwrap();
        assert!(decoder.is_done());

        assert_eq!(decoder.output(), b"Hello");
    }

    #[test]
    fn test_incremental_byte_by_byte() {
        let mut decoder = ChunkedDecoder::new();
        let data = b"3\r\nABC\r\n0\r\n\r\n";

        for &byte in data.iter() {
            decoder.feed(&[byte]).unwrap();
        }

        assert!(decoder.is_done());
        assert_eq!(decoder.output(), b"ABC");
    }

    // ==================== State Checks ====================

    #[test]
    fn test_initial_state() {
        let decoder = ChunkedDecoder::new();
        assert_eq!(decoder.state(), DecoderState::ReadingSize);
        assert!(!decoder.is_done());
    }

    #[test]
    fn test_done_state() {
        let data = b"0\r\n\r\n";
        let mut decoder = ChunkedDecoder::new();
        decoder.feed(data).unwrap();

        assert_eq!(decoder.state(), DecoderState::Done);
        assert!(decoder.is_done());
    }

    #[test]
    fn test_reset() {
        let mut decoder = ChunkedDecoder::new();
        decoder.feed(b"5\r\nHello\r\n0\r\n\r\n").unwrap();

        assert!(decoder.is_done());
        assert!(!decoder.output().is_empty());

        decoder.reset();

        assert!(!decoder.is_done());
        assert!(decoder.output().is_empty());
        assert_eq!(decoder.state(), DecoderState::ReadingSize);
    }

    // ==================== Take Output ====================

    #[test]
    fn test_take_output() {
        let data = b"5\r\nHello\r\n0\r\n\r\n";
        let mut decoder = ChunkedDecoder::new();
        decoder.feed(data).unwrap();

        let output = decoder.take_output();
        assert_eq!(output, b"Hello");
    }

    // ==================== Error Cases ====================

    #[test]
    fn test_decode_incomplete() {
        let data = b"5\r\nHel"; // Missing rest
        let result = ChunkedDecoder::decode(data);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_hex() {
        let data = b"XYZ\r\ndata\r\n0\r\n\r\n";
        let result = ChunkedDecoder::decode(data);
        assert!(result.is_err());
    }

    // ==================== Real-World Data ====================

    #[test]
    fn test_typical_response() {
        // Simulates a typical chunked HTML response
        // 0x1f = 31 bytes for first chunk
        let chunk1 = b"<!DOCTYPE html>\n<html><body>";
        let chunk2 = b"</body></html>";

        let mut data = Vec::new();
        data.extend_from_slice(format!("{:x}\r\n", chunk1.len()).as_bytes());
        data.extend_from_slice(chunk1);
        data.extend_from_slice(b"\r\n");
        data.extend_from_slice(format!("{:x}\r\n", chunk2.len()).as_bytes());
        data.extend_from_slice(chunk2);
        data.extend_from_slice(b"\r\n0\r\n\r\n");

        let result = ChunkedDecoder::decode(&data).unwrap();
        let html = core::str::from_utf8(&result).unwrap();
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.ends_with("</body></html>"));
    }

    #[test]
    fn test_binary_data() {
        // Binary data with null bytes
        let chunk_data = [0x00, 0xFF, 0x7F, 0x80, 0x01];
        let mut data = Vec::new();
        data.extend_from_slice(b"5\r\n");
        data.extend_from_slice(&chunk_data);
        data.extend_from_slice(b"\r\n0\r\n\r\n");

        let result = ChunkedDecoder::decode(&data).unwrap();
        assert_eq!(result, chunk_data);
    }
}
