//! Buffer management utilities for network operations.
//!
//! Provides efficient buffer types for streaming data:
//! - `RingBuffer` - Circular buffer for producer/consumer patterns
//! - `ChunkBuffer` - Accumulator for incomplete HTTP chunks
//! - `SliceReader` - Zero-copy reading from byte slices
//!
//! # Design Goals
//!
//! - Zero-copy where possible
//! - Predictable memory usage
//! - no_std compatible
//!
//! # Examples
//!
//! ```ignore
//! use morpheus_network::utils::buffer::RingBuffer;
//!
//! let mut buffer = RingBuffer::new(1024);
//! buffer.write(b"Hello").unwrap();
//! let mut out = [0u8; 5];
//! buffer.read(&mut out).unwrap();
//! assert_eq!(&out, b"Hello");
//! ```

use alloc::vec::Vec;
use alloc::vec;

/// Error type for buffer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferError {
    /// Buffer is full, cannot write more data.
    Full,
    /// Buffer is empty, cannot read data.
    Empty,
    /// Requested operation exceeds available space/data.
    InsufficientSpace,
    /// Invalid parameter provided.
    InvalidParameter,
}

/// Result type for buffer operations.
pub type BufferResult<T> = Result<T, BufferError>;

/// Ring buffer for efficient FIFO data handling.
///
/// Provides O(1) push/pop operations with wraparound.
/// Useful for streaming network data where input rate varies.
#[derive(Debug)]
pub struct RingBuffer {
    /// Internal storage.
    data: Vec<u8>,
    /// Read position (head).
    read_pos: usize,
    /// Write position (tail).
    write_pos: usize,
    /// Number of bytes currently stored.
    len: usize,
}

impl RingBuffer {
    /// Create a new ring buffer with the given capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum bytes the buffer can hold
    ///
    /// # Panics
    ///
    /// Panics if capacity is 0.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "Buffer capacity must be > 0");
        Self {
            data: vec![0u8; capacity],
            read_pos: 0,
            write_pos: 0,
            len: 0,
        }
    }

    /// Returns the buffer's capacity.
    pub fn capacity(&self) -> usize {
        self.data.len()
    }

    /// Returns number of bytes currently stored.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns true if buffer is full.
    pub fn is_full(&self) -> bool {
        self.len == self.capacity()
    }

    /// Returns available space for writing.
    pub fn available(&self) -> usize {
        self.capacity() - self.len
    }

    /// Write data to the buffer.
    ///
    /// # Arguments
    ///
    /// * `data` - Bytes to write
    ///
    /// # Returns
    ///
    /// Number of bytes written (may be less than data.len() if buffer fills).
    pub fn write(&mut self, data: &[u8]) -> usize {
        let to_write = data.len().min(self.available());
        
        for &byte in &data[..to_write] {
            self.data[self.write_pos] = byte;
            self.write_pos = (self.write_pos + 1) % self.capacity();
            self.len += 1;
        }
        
        to_write
    }

    /// Write all data or return error if insufficient space.
    pub fn write_all(&mut self, data: &[u8]) -> BufferResult<()> {
        if data.len() > self.available() {
            return Err(BufferError::InsufficientSpace);
        }
        self.write(data);
        Ok(())
    }

    /// Read data from the buffer.
    ///
    /// # Arguments
    ///
    /// * `buf` - Destination buffer
    ///
    /// # Returns
    ///
    /// Number of bytes read (may be less than buf.len() if buffer empties).
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let to_read = buf.len().min(self.len);
        
        for byte in buf.iter_mut().take(to_read) {
            *byte = self.data[self.read_pos];
            self.read_pos = (self.read_pos + 1) % self.capacity();
            self.len -= 1;
        }
        
        to_read
    }

    /// Read exactly n bytes or return error.
    pub fn read_exact(&mut self, buf: &mut [u8]) -> BufferResult<()> {
        if buf.len() > self.len {
            return Err(BufferError::InsufficientSpace);
        }
        self.read(buf);
        Ok(())
    }

    /// Peek at data without consuming it.
    ///
    /// # Arguments
    ///
    /// * `buf` - Destination buffer
    ///
    /// # Returns
    ///
    /// Number of bytes peeked.
    pub fn peek(&self, buf: &mut [u8]) -> usize {
        let to_peek = buf.len().min(self.len);
        let mut pos = self.read_pos;
        
        for byte in buf.iter_mut().take(to_peek) {
            *byte = self.data[pos];
            pos = (pos + 1) % self.capacity();
        }
        
        to_peek
    }

    /// Skip n bytes without reading them.
    pub fn skip(&mut self, n: usize) -> usize {
        let to_skip = n.min(self.len);
        self.read_pos = (self.read_pos + to_skip) % self.capacity();
        self.len -= to_skip;
        to_skip
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.read_pos = 0;
        self.write_pos = 0;
        self.len = 0;
    }

    /// Read a single byte.
    pub fn read_byte(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        let byte = self.data[self.read_pos];
        self.read_pos = (self.read_pos + 1) % self.capacity();
        self.len -= 1;
        Some(byte)
    }

    /// Write a single byte.
    pub fn write_byte(&mut self, byte: u8) -> BufferResult<()> {
        if self.is_full() {
            return Err(BufferError::Full);
        }
        self.data[self.write_pos] = byte;
        self.write_pos = (self.write_pos + 1) % self.capacity();
        self.len += 1;
        Ok(())
    }
}

/// Chunk buffer for accumulating incomplete data.
///
/// Used when receiving chunked HTTP responses where chunk
/// boundaries don't align with read boundaries.
#[derive(Debug)]
pub struct ChunkBuffer {
    /// Accumulated data.
    data: Vec<u8>,
    /// Maximum size to prevent unbounded growth.
    max_size: usize,
}

impl ChunkBuffer {
    /// Create a new chunk buffer.
    ///
    /// # Arguments
    ///
    /// * `max_size` - Maximum accumulated size (prevents OOM)
    pub fn new(max_size: usize) -> Self {
        Self {
            data: Vec::new(),
            max_size,
        }
    }

    /// Append data to the buffer.
    pub fn append(&mut self, data: &[u8]) -> BufferResult<()> {
        if self.data.len() + data.len() > self.max_size {
            return Err(BufferError::InsufficientSpace);
        }
        self.data.extend_from_slice(data);
        Ok(())
    }

    /// Get the accumulated data.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Take ownership of accumulated data, clearing the buffer.
    pub fn take(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.data)
    }

    /// Get length of accumulated data.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Consume n bytes from the front.
    pub fn consume(&mut self, n: usize) {
        if n >= self.data.len() {
            self.data.clear();
        } else {
            self.data.drain(..n);
        }
    }

    /// Find a pattern in the buffer.
    pub fn find(&self, pattern: &[u8]) -> Option<usize> {
        if pattern.is_empty() || pattern.len() > self.data.len() {
            return None;
        }
        
        self.data
            .windows(pattern.len())
            .position(|window| window == pattern)
    }

    /// Find CRLF sequence.
    pub fn find_crlf(&self) -> Option<usize> {
        self.find(b"\r\n")
    }

    /// Check if buffer starts with pattern.
    pub fn starts_with(&self, pattern: &[u8]) -> bool {
        self.data.starts_with(pattern)
    }
}

/// Zero-copy slice reader.
///
/// Tracks position in a byte slice without copying data.
#[derive(Debug)]
pub struct SliceReader<'a> {
    /// Source data.
    data: &'a [u8],
    /// Current position.
    pos: usize,
}

impl<'a> SliceReader<'a> {
    /// Create a new slice reader.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Get remaining bytes.
    pub fn remaining(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    /// Get number of remaining bytes.
    pub fn remaining_len(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Check if all data has been consumed.
    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// Get current position.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Read up to n bytes.
    pub fn read(&mut self, n: usize) -> &'a [u8] {
        let end = (self.pos + n).min(self.data.len());
        let result = &self.data[self.pos..end];
        self.pos = end;
        result
    }

    /// Read exactly n bytes or None.
    pub fn read_exact(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.remaining_len() < n {
            return None;
        }
        Some(self.read(n))
    }

    /// Peek at next n bytes without advancing.
    pub fn peek(&self, n: usize) -> &'a [u8] {
        let end = (self.pos + n).min(self.data.len());
        &self.data[self.pos..end]
    }

    /// Skip n bytes.
    pub fn skip(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.data.len());
    }

    /// Read a single byte.
    pub fn read_byte(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        let byte = self.data[self.pos];
        self.pos += 1;
        Some(byte)
    }

    /// Peek at next byte.
    pub fn peek_byte(&self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        Some(self.data[self.pos])
    }

    /// Read until a delimiter (exclusive).
    pub fn read_until(&mut self, delimiter: u8) -> Option<&'a [u8]> {
        let remaining = self.remaining();
        if let Some(idx) = remaining.iter().position(|&b| b == delimiter) {
            let result = &remaining[..idx];
            self.pos += idx;
            Some(result)
        } else {
            None
        }
    }

    /// Read a line (until \n, not including \r\n).
    pub fn read_line(&mut self) -> Option<&'a [u8]> {
        let remaining = self.remaining();
        
        // Find \n
        if let Some(lf_pos) = remaining.iter().position(|&b| b == b'\n') {
            // Check for \r before \n
            let end = if lf_pos > 0 && remaining[lf_pos - 1] == b'\r' {
                lf_pos - 1
            } else {
                lf_pos
            };
            
            let result = &remaining[..end];
            self.pos += lf_pos + 1; // Skip past \n
            Some(result)
        } else {
            None
        }
    }

    /// Reset to beginning.
    pub fn reset(&mut self) {
        self.pos = 0;
    }

    /// Get all data (ignoring position).
    pub fn all_data(&self) -> &'a [u8] {
        self.data
    }
}

/// Simple growable buffer with efficient appends.
#[derive(Debug, Clone)]
pub struct GrowableBuffer {
    data: Vec<u8>,
    max_size: Option<usize>,
}

impl GrowableBuffer {
    /// Create a new growable buffer.
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            max_size: None,
        }
    }

    /// Create with initial capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            max_size: None,
        }
    }

    /// Create with maximum size limit.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            data: Vec::new(),
            max_size: Some(max_size),
        }
    }

    /// Append data.
    pub fn append(&mut self, data: &[u8]) -> BufferResult<()> {
        if let Some(max) = self.max_size {
            if self.data.len() + data.len() > max {
                return Err(BufferError::InsufficientSpace);
            }
        }
        self.data.extend_from_slice(data);
        Ok(())
    }

    /// Get data as slice.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Take ownership of data.
    pub fn take(self) -> Vec<u8> {
        self.data
    }

    /// Get length.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Clear buffer.
    pub fn clear(&mut self) {
        self.data.clear();
    }
}

impl Default for GrowableBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== RingBuffer Tests ====================

    #[test]
    fn test_ring_buffer_new() {
        let buf = RingBuffer::new(100);
        assert_eq!(buf.capacity(), 100);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert!(!buf.is_full());
    }

    #[test]
    #[should_panic]
    fn test_ring_buffer_zero_capacity() {
        RingBuffer::new(0);
    }

    #[test]
    fn test_ring_buffer_write_read() {
        let mut buf = RingBuffer::new(100);
        
        let written = buf.write(b"Hello");
        assert_eq!(written, 5);
        assert_eq!(buf.len(), 5);
        
        let mut out = [0u8; 5];
        let read = buf.read(&mut out);
        assert_eq!(read, 5);
        assert_eq!(&out, b"Hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn test_ring_buffer_partial_read() {
        let mut buf = RingBuffer::new(100);
        buf.write(b"Hello World");
        
        let mut out = [0u8; 5];
        buf.read(&mut out);
        assert_eq!(&out, b"Hello");
        assert_eq!(buf.len(), 6); // " World" remains
    }

    #[test]
    fn test_ring_buffer_wraparound() {
        let mut buf = RingBuffer::new(10);
        
        // Fill most of the buffer
        buf.write(b"12345678");
        assert_eq!(buf.len(), 8);
        
        // Read some
        let mut out = [0u8; 5];
        buf.read(&mut out);
        assert_eq!(&out, b"12345");
        assert_eq!(buf.len(), 3);
        
        // Write more (will wrap around)
        buf.write(b"ABCDE");
        assert_eq!(buf.len(), 8);
        
        // Read all
        let mut out2 = [0u8; 8];
        buf.read(&mut out2);
        assert_eq!(&out2, b"678ABCDE");
    }

    #[test]
    fn test_ring_buffer_full() {
        let mut buf = RingBuffer::new(5);
        
        let written = buf.write(b"12345");
        assert_eq!(written, 5);
        assert!(buf.is_full());
        
        // Try to write more
        let written = buf.write(b"67");
        assert_eq!(written, 0);
    }

    #[test]
    fn test_ring_buffer_peek() {
        let mut buf = RingBuffer::new(100);
        buf.write(b"Hello");
        
        let mut out = [0u8; 3];
        let peeked = buf.peek(&mut out);
        assert_eq!(peeked, 3);
        assert_eq!(&out, b"Hel");
        assert_eq!(buf.len(), 5); // Still 5, not consumed
    }

    #[test]
    fn test_ring_buffer_skip() {
        let mut buf = RingBuffer::new(100);
        buf.write(b"Hello World");
        
        buf.skip(6);
        assert_eq!(buf.len(), 5);
        
        let mut out = [0u8; 5];
        buf.read(&mut out);
        assert_eq!(&out, b"World");
    }

    #[test]
    fn test_ring_buffer_byte_ops() {
        let mut buf = RingBuffer::new(10);
        
        buf.write_byte(b'A').unwrap();
        buf.write_byte(b'B').unwrap();
        
        assert_eq!(buf.read_byte(), Some(b'A'));
        assert_eq!(buf.read_byte(), Some(b'B'));
        assert_eq!(buf.read_byte(), None);
    }

    #[test]
    fn test_ring_buffer_clear() {
        let mut buf = RingBuffer::new(100);
        buf.write(b"Hello");
        buf.clear();
        
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    // ==================== ChunkBuffer Tests ====================

    #[test]
    fn test_chunk_buffer_append() {
        let mut buf = ChunkBuffer::new(1000);
        
        buf.append(b"Hello ").unwrap();
        buf.append(b"World").unwrap();
        
        assert_eq!(buf.data(), b"Hello World");
        assert_eq!(buf.len(), 11);
    }

    #[test]
    fn test_chunk_buffer_max_size() {
        let mut buf = ChunkBuffer::new(10);
        
        buf.append(b"12345").unwrap();
        assert!(buf.append(b"67890X").is_err()); // Would exceed max
    }

    #[test]
    fn test_chunk_buffer_take() {
        let mut buf = ChunkBuffer::new(1000);
        buf.append(b"Hello").unwrap();
        
        let data = buf.take();
        assert_eq!(data, b"Hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn test_chunk_buffer_consume() {
        let mut buf = ChunkBuffer::new(1000);
        buf.append(b"Hello World").unwrap();
        
        buf.consume(6);
        assert_eq!(buf.data(), b"World");
    }

    #[test]
    fn test_chunk_buffer_find() {
        let mut buf = ChunkBuffer::new(1000);
        buf.append(b"Hello\r\nWorld").unwrap();
        
        assert_eq!(buf.find(b"\r\n"), Some(5));
        assert_eq!(buf.find_crlf(), Some(5));
        assert_eq!(buf.find(b"xyz"), None);
    }

    #[test]
    fn test_chunk_buffer_starts_with() {
        let mut buf = ChunkBuffer::new(1000);
        buf.append(b"HTTP/1.1 200").unwrap();
        
        assert!(buf.starts_with(b"HTTP"));
        assert!(!buf.starts_with(b"FTP"));
    }

    // ==================== SliceReader Tests ====================

    #[test]
    fn test_slice_reader_basic() {
        let data = b"Hello World";
        let mut reader = SliceReader::new(data);
        
        assert_eq!(reader.remaining_len(), 11);
        assert!(!reader.is_empty());
        
        let chunk = reader.read(5);
        assert_eq!(chunk, b"Hello");
        assert_eq!(reader.position(), 5);
        assert_eq!(reader.remaining(), b" World");
    }

    #[test]
    fn test_slice_reader_read_exact() {
        let data = b"Hello";
        let mut reader = SliceReader::new(data);
        
        assert!(reader.read_exact(5).is_some());
        assert!(reader.read_exact(1).is_none()); // Nothing left
    }

    #[test]
    fn test_slice_reader_peek() {
        let data = b"Hello";
        let mut reader = SliceReader::new(data);
        
        assert_eq!(reader.peek(3), b"Hel");
        assert_eq!(reader.position(), 0); // Not advanced
        
        reader.skip(2);
        assert_eq!(reader.peek(3), b"llo");
    }

    #[test]
    fn test_slice_reader_byte_ops() {
        let data = b"ABC";
        let mut reader = SliceReader::new(data);
        
        assert_eq!(reader.peek_byte(), Some(b'A'));
        assert_eq!(reader.read_byte(), Some(b'A'));
        assert_eq!(reader.read_byte(), Some(b'B'));
        assert_eq!(reader.read_byte(), Some(b'C'));
        assert_eq!(reader.read_byte(), None);
    }

    #[test]
    fn test_slice_reader_read_until() {
        let data = b"key=value&other";
        let mut reader = SliceReader::new(data);
        
        let key = reader.read_until(b'=');
        assert_eq!(key, Some(b"key".as_slice()));
        
        reader.skip(1); // Skip '='
        
        let value = reader.read_until(b'&');
        assert_eq!(value, Some(b"value".as_slice()));
    }

    #[test]
    fn test_slice_reader_read_line() {
        let data = b"Line1\r\nLine2\r\nLine3";
        let mut reader = SliceReader::new(data);
        
        assert_eq!(reader.read_line(), Some(b"Line1".as_slice()));
        assert_eq!(reader.read_line(), Some(b"Line2".as_slice()));
        assert_eq!(reader.read_line(), None); // No more complete lines
    }

    #[test]
    fn test_slice_reader_read_line_lf_only() {
        let data = b"Line1\nLine2\n";
        let mut reader = SliceReader::new(data);
        
        assert_eq!(reader.read_line(), Some(b"Line1".as_slice()));
        assert_eq!(reader.read_line(), Some(b"Line2".as_slice()));
    }

    #[test]
    fn test_slice_reader_reset() {
        let data = b"Hello";
        let mut reader = SliceReader::new(data);
        
        reader.read(3);
        assert_eq!(reader.position(), 3);
        
        reader.reset();
        assert_eq!(reader.position(), 0);
    }

    // ==================== GrowableBuffer Tests ====================

    #[test]
    fn test_growable_buffer_basic() {
        let mut buf = GrowableBuffer::new();
        
        buf.append(b"Hello").unwrap();
        buf.append(b" World").unwrap();
        
        assert_eq!(buf.as_slice(), b"Hello World");
        assert_eq!(buf.len(), 11);
    }

    #[test]
    fn test_growable_buffer_max_size() {
        let mut buf = GrowableBuffer::with_max_size(10);
        
        buf.append(b"12345").unwrap();
        assert!(buf.append(b"67890X").is_err());
    }

    #[test]
    fn test_growable_buffer_take() {
        let mut buf = GrowableBuffer::new();
        buf.append(b"Hello").unwrap();
        
        let data = buf.take();
        assert_eq!(data, b"Hello");
    }

    #[test]
    fn test_growable_buffer_clear() {
        let mut buf = GrowableBuffer::new();
        buf.append(b"Hello").unwrap();
        buf.clear();
        
        assert!(buf.is_empty());
    }
}
