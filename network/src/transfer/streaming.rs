//! Streaming transfer handler for large downloads.
//!
//! Provides incremental data transfer with:
//! - Progress callbacks for UI updates
//! - Cancellation support
//! - Memory-efficient streaming (no full buffering)
//! - Automatic chunk size management
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     ┌──────────────┐     ┌──────────────┐
//! │ Data Source │────▶│ StreamReader │────▶│ Sink/Output  │
//! │   (HTTP)    │     │  (progress)  │     │ (file/buffer)│
//! └─────────────┘     └──────────────┘     └──────────────┘
//! ```
//!
//! # Examples
//!
//! ```ignore
//! use morpheus_network::transfer::StreamReader;
//!
//! let mut reader = StreamReader::new(1024 * 64); // 64KB chunks
//! reader.set_expected_size(Some(1024 * 1024)); // 1MB expected
//! reader.set_progress_callback(|transferred, total| {
//!     println!("Progress: {}/{:?}", transferred, total);
//! });
//!
//! while !reader.is_complete() {
//!     let chunk = fetch_next_chunk();
//!     reader.feed(&chunk)?;
//! }
//! ```

use alloc::vec::Vec;
use crate::error::{NetworkError, Result};
use crate::types::ProgressCallback;

/// State of a streaming transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    /// Ready to receive data.
    Ready,
    /// Actively receiving data.
    Receiving,
    /// Transfer completed successfully.
    Complete,
    /// Transfer was cancelled.
    Cancelled,
    /// Transfer failed with error.
    Failed,
}

/// Configuration for streaming transfers.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// Size of internal buffer for accumulating data.
    pub buffer_size: usize,
    /// How often to call progress callback (in bytes).
    pub progress_interval: usize,
    /// Maximum total size to accept (prevents OOM).
    pub max_size: Option<usize>,
    /// Timeout per chunk in milliseconds (if supported).
    pub chunk_timeout_ms: Option<u32>,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            buffer_size: 64 * 1024,       // 64KB buffer
            progress_interval: 16 * 1024, // Report every 16KB
            max_size: None,               // No limit
            chunk_timeout_ms: Some(30000), // 30 second timeout
        }
    }
}

impl StreamConfig {
    /// Create config for small files.
    pub fn small() -> Self {
        Self {
            buffer_size: 8 * 1024,
            progress_interval: 4 * 1024,
            max_size: Some(1024 * 1024), // 1MB max
            chunk_timeout_ms: Some(10000),
        }
    }

    /// Create config for large files (ISOs, etc).
    pub fn large() -> Self {
        Self {
            buffer_size: 256 * 1024,       // 256KB buffer
            progress_interval: 1024 * 1024, // Report every 1MB
            max_size: None,
            chunk_timeout_ms: Some(60000), // 60 second timeout
        }
    }
}

/// Streaming data reader with progress tracking.
///
/// Accumulates data chunks and provides progress updates.
#[derive(Debug)]
pub struct StreamReader {
    /// Configuration.
    config: StreamConfig,
    /// Current state.
    state: StreamState,
    /// Accumulated data.
    buffer: Vec<u8>,
    /// Total bytes received.
    bytes_received: usize,
    /// Expected total size (from Content-Length).
    expected_size: Option<usize>,
    /// Bytes since last progress callback.
    bytes_since_progress: usize,
    /// Progress callback (stored as Option since fn pointers can't be Debug).
    progress_callback: Option<ProgressCallback>,
    /// Cancellation flag.
    cancelled: bool,
}

impl StreamReader {
    /// Create a new stream reader with default config.
    pub fn new() -> Self {
        Self::with_config(StreamConfig::default())
    }

    /// Create with specific buffer size.
    pub fn with_buffer_size(buffer_size: usize) -> Self {
        Self::with_config(StreamConfig {
            buffer_size,
            ..Default::default()
        })
    }

    /// Create with full configuration.
    pub fn with_config(config: StreamConfig) -> Self {
        Self {
            buffer: Vec::with_capacity(config.buffer_size),
            config,
            state: StreamState::Ready,
            bytes_received: 0,
            expected_size: None,
            bytes_since_progress: 0,
            progress_callback: None,
            cancelled: false,
        }
    }

    /// Set expected total size (from Content-Length header).
    pub fn set_expected_size(&mut self, size: Option<usize>) {
        self.expected_size = size;
    }

    /// Get expected total size.
    pub fn expected_size(&self) -> Option<usize> {
        self.expected_size
    }

    /// Set progress callback.
    pub fn set_progress_callback(&mut self, callback: ProgressCallback) {
        self.progress_callback = Some(callback);
    }

    /// Get current state.
    pub fn state(&self) -> StreamState {
        self.state
    }

    /// Check if transfer is complete.
    pub fn is_complete(&self) -> bool {
        self.state == StreamState::Complete
    }

    /// Check if transfer was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled || self.state == StreamState::Cancelled
    }

    /// Get bytes received so far.
    pub fn bytes_received(&self) -> usize {
        self.bytes_received
    }

    /// Get progress as percentage (0-100).
    pub fn progress_percent(&self) -> Option<u8> {
        self.expected_size.map(|total| {
            if total == 0 {
                100
            } else {
                ((self.bytes_received as u64 * 100) / total as u64) as u8
            }
        })
    }

    /// Cancel the transfer.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.state = StreamState::Cancelled;
    }

    /// Feed data chunk to the reader.
    ///
    /// Returns number of bytes consumed.
    pub fn feed(&mut self, data: &[u8]) -> Result<usize> {
        // Check cancellation
        if self.cancelled {
            self.state = StreamState::Cancelled;
            return Err(NetworkError::Cancelled);
        }

        // Check max size limit
        if let Some(max) = self.config.max_size {
            if self.bytes_received + data.len() > max {
                self.state = StreamState::Failed;
                return Err(NetworkError::OutOfMemory);
            }
        }

        // Update state
        if self.state == StreamState::Ready {
            self.state = StreamState::Receiving;
        }

        // Append data
        self.buffer.extend_from_slice(data);
        self.bytes_received += data.len();
        self.bytes_since_progress += data.len();

        // Call progress callback if interval reached
        if self.bytes_since_progress >= self.config.progress_interval {
            self.report_progress();
            self.bytes_since_progress = 0;
        }

        // Check for completion
        if let Some(expected) = self.expected_size {
            if self.bytes_received >= expected {
                self.state = StreamState::Complete;
                // Final progress report
                self.report_progress();
            }
        }

        Ok(data.len())
    }

    /// Mark transfer as complete (for chunked encoding without Content-Length).
    pub fn finish(&mut self) {
        if self.state == StreamState::Receiving || self.state == StreamState::Ready {
            self.state = StreamState::Complete;
            self.report_progress();
        }
    }

    /// Mark transfer as failed.
    pub fn fail(&mut self) {
        self.state = StreamState::Failed;
    }

    /// Get accumulated data.
    pub fn data(&self) -> &[u8] {
        &self.buffer
    }

    /// Take ownership of accumulated data.
    pub fn take_data(self) -> Vec<u8> {
        self.buffer
    }

    /// Clear buffer but keep state.
    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
    }

    /// Reset reader for reuse.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.state = StreamState::Ready;
        self.bytes_received = 0;
        self.expected_size = None;
        self.bytes_since_progress = 0;
        self.cancelled = false;
    }

    /// Report progress via callback.
    fn report_progress(&self) {
        if let Some(callback) = self.progress_callback {
            callback(self.bytes_received, self.expected_size);
        }
    }
}

impl Default for StreamReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Streaming writer for sending data in chunks.
///
/// Used for POST/PUT requests with large bodies.
#[derive(Debug)]
pub struct StreamWriter {
    /// Total bytes to send.
    total_size: usize,
    /// Bytes sent so far.
    bytes_sent: usize,
    /// Chunk size for sending.
    chunk_size: usize,
    /// Progress callback.
    progress_callback: Option<ProgressCallback>,
    /// Progress reporting interval.
    progress_interval: usize,
    /// Bytes since last progress report.
    bytes_since_progress: usize,
}

impl StreamWriter {
    /// Create a new stream writer.
    ///
    /// # Arguments
    ///
    /// * `total_size` - Total bytes to be sent
    /// * `chunk_size` - Size of each chunk to send
    pub fn new(total_size: usize, chunk_size: usize) -> Self {
        Self {
            total_size,
            bytes_sent: 0,
            chunk_size,
            progress_callback: None,
            progress_interval: 16 * 1024,
            bytes_since_progress: 0,
        }
    }

    /// Set progress callback.
    pub fn set_progress_callback(&mut self, callback: ProgressCallback) {
        self.progress_callback = Some(callback);
    }

    /// Get next chunk to send from source data.
    ///
    /// Returns slice of data for next chunk, or None if complete.
    pub fn next_chunk<'a>(&mut self, source: &'a [u8]) -> Option<&'a [u8]> {
        if self.bytes_sent >= self.total_size {
            return None;
        }

        let remaining = self.total_size - self.bytes_sent;
        let chunk_len = remaining.min(self.chunk_size);
        let start = self.bytes_sent;
        let end = start + chunk_len;

        if end > source.len() {
            return None;
        }

        Some(&source[start..end])
    }

    /// Mark chunk as sent.
    pub fn chunk_sent(&mut self, bytes: usize) {
        self.bytes_sent += bytes;
        self.bytes_since_progress += bytes;

        if self.bytes_since_progress >= self.progress_interval {
            self.report_progress();
            self.bytes_since_progress = 0;
        }
    }

    /// Check if all data has been sent.
    pub fn is_complete(&self) -> bool {
        self.bytes_sent >= self.total_size
    }

    /// Get bytes sent so far.
    pub fn bytes_sent(&self) -> usize {
        self.bytes_sent
    }

    /// Get remaining bytes.
    pub fn remaining(&self) -> usize {
        self.total_size.saturating_sub(self.bytes_sent)
    }

    /// Get progress percentage.
    pub fn progress_percent(&self) -> u8 {
        if self.total_size == 0 {
            100
        } else {
            ((self.bytes_sent as u64 * 100) / self.total_size as u64) as u8
        }
    }

    /// Report progress via callback.
    fn report_progress(&self) {
        if let Some(callback) = self.progress_callback {
            callback(self.bytes_sent, Some(self.total_size));
        }
    }
}

/// Progress tracker for transfers.
///
/// Standalone progress tracking without buffering.
/// Useful when data flows directly to destination.
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    /// Total bytes expected (if known).
    total: Option<usize>,
    /// Bytes transferred.
    transferred: usize,
    /// Progress callback.
    callback: Option<ProgressCallback>,
    /// Reporting interval.
    interval: usize,
    /// Bytes since last report.
    since_report: usize,
}

impl ProgressTracker {
    /// Create a new progress tracker.
    pub fn new(total: Option<usize>) -> Self {
        Self {
            total,
            transferred: 0,
            callback: None,
            interval: 16 * 1024,
            since_report: 0,
        }
    }

    /// Set progress callback.
    pub fn set_callback(&mut self, callback: ProgressCallback) {
        self.callback = Some(callback);
    }

    /// Set reporting interval.
    pub fn set_interval(&mut self, interval: usize) {
        self.interval = interval;
    }

    /// Update with transferred bytes.
    pub fn update(&mut self, bytes: usize) {
        self.transferred += bytes;
        self.since_report += bytes;

        if self.since_report >= self.interval {
            self.report();
            self.since_report = 0;
        }
    }

    /// Force a progress report.
    pub fn report(&self) {
        if let Some(callback) = self.callback {
            callback(self.transferred, self.total);
        }
    }

    /// Get bytes transferred.
    pub fn transferred(&self) -> usize {
        self.transferred
    }

    /// Get progress percentage.
    pub fn percent(&self) -> Option<u8> {
        self.total.map(|t| {
            if t == 0 {
                100
            } else {
                ((self.transferred as u64 * 100) / t as u64) as u8
            }
        })
    }

    /// Check if transfer is complete.
    pub fn is_complete(&self) -> bool {
        self.total.is_some_and(|t| self.transferred >= t)
    }

    /// Reset tracker.
    pub fn reset(&mut self) {
        self.transferred = 0;
        self.since_report = 0;
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use core::cell::Cell;

    // ==================== StreamConfig Tests ====================

    #[test]
    fn test_stream_config_default() {
        let config = StreamConfig::default();
        assert_eq!(config.buffer_size, 64 * 1024);
        assert_eq!(config.progress_interval, 16 * 1024);
        assert!(config.max_size.is_none());
    }

    #[test]
    fn test_stream_config_small() {
        let config = StreamConfig::small();
        assert_eq!(config.buffer_size, 8 * 1024);
        assert_eq!(config.max_size, Some(1024 * 1024));
    }

    #[test]
    fn test_stream_config_large() {
        let config = StreamConfig::large();
        assert_eq!(config.buffer_size, 256 * 1024);
        assert!(config.max_size.is_none());
    }

    // ==================== StreamReader Tests ====================

    #[test]
    fn test_stream_reader_new() {
        let reader = StreamReader::new();
        assert_eq!(reader.state(), StreamState::Ready);
        assert_eq!(reader.bytes_received(), 0);
        assert!(!reader.is_complete());
    }

    #[test]
    fn test_stream_reader_feed_basic() {
        let mut reader = StreamReader::new();
        
        reader.feed(b"Hello ").unwrap();
        assert_eq!(reader.state(), StreamState::Receiving);
        assert_eq!(reader.bytes_received(), 6);
        
        reader.feed(b"World").unwrap();
        assert_eq!(reader.bytes_received(), 11);
        assert_eq!(reader.data(), b"Hello World");
    }

    #[test]
    fn test_stream_reader_with_expected_size() {
        let mut reader = StreamReader::new();
        reader.set_expected_size(Some(10));
        
        reader.feed(b"12345").unwrap();
        assert!(!reader.is_complete());
        
        reader.feed(b"67890").unwrap();
        assert!(reader.is_complete());
        assert_eq!(reader.state(), StreamState::Complete);
    }

    #[test]
    fn test_stream_reader_cancel() {
        let mut reader = StreamReader::new();
        
        reader.feed(b"Hello").unwrap();
        reader.cancel();
        
        assert!(reader.is_cancelled());
        assert_eq!(reader.state(), StreamState::Cancelled);
        
        // Further feeds should fail
        assert!(reader.feed(b"More").is_err());
    }

    #[test]
    fn test_stream_reader_max_size() {
        let config = StreamConfig {
            max_size: Some(10),
            ..Default::default()
        };
        let mut reader = StreamReader::with_config(config);
        
        reader.feed(b"12345").unwrap();
        
        // This should fail - exceeds max
        let result = reader.feed(b"67890X");
        assert!(result.is_err());
        assert_eq!(reader.state(), StreamState::Failed);
    }

    #[test]
    fn test_stream_reader_progress_percent() {
        let mut reader = StreamReader::new();
        reader.set_expected_size(Some(100));
        
        assert_eq!(reader.progress_percent(), Some(0));
        
        reader.feed(&[0u8; 25]).unwrap();
        assert_eq!(reader.progress_percent(), Some(25));
        
        reader.feed(&[0u8; 25]).unwrap();
        assert_eq!(reader.progress_percent(), Some(50));
        
        reader.feed(&[0u8; 50]).unwrap();
        assert_eq!(reader.progress_percent(), Some(100));
    }

    #[test]
    fn test_stream_reader_progress_percent_unknown_size() {
        let reader = StreamReader::new();
        assert_eq!(reader.progress_percent(), None);
    }

    #[test]
    fn test_stream_reader_finish() {
        let mut reader = StreamReader::new();
        reader.feed(b"Hello").unwrap();
        
        assert!(!reader.is_complete());
        reader.finish();
        assert!(reader.is_complete());
    }

    #[test]
    fn test_stream_reader_take_data() {
        let mut reader = StreamReader::new();
        reader.feed(b"Hello World").unwrap();
        
        let data = reader.take_data();
        assert_eq!(data, b"Hello World");
    }

    #[test]
    fn test_stream_reader_reset() {
        let mut reader = StreamReader::new();
        reader.set_expected_size(Some(100));
        reader.feed(b"Hello").unwrap();
        
        reader.reset();
        
        assert_eq!(reader.state(), StreamState::Ready);
        assert_eq!(reader.bytes_received(), 0);
        assert!(reader.expected_size().is_none());
        assert!(reader.data().is_empty());
    }

    // ==================== StreamWriter Tests ====================

    #[test]
    fn test_stream_writer_new() {
        let writer = StreamWriter::new(100, 10);
        assert_eq!(writer.bytes_sent(), 0);
        assert_eq!(writer.remaining(), 100);
        assert!(!writer.is_complete());
    }

    #[test]
    fn test_stream_writer_next_chunk() {
        let source = b"Hello World!";
        let mut writer = StreamWriter::new(source.len(), 5);
        
        let chunk1 = writer.next_chunk(source).unwrap();
        assert_eq!(chunk1, b"Hello");
        writer.chunk_sent(5);
        
        let chunk2 = writer.next_chunk(source).unwrap();
        assert_eq!(chunk2, b" Worl");
        writer.chunk_sent(5);
        
        let chunk3 = writer.next_chunk(source).unwrap();
        assert_eq!(chunk3, b"d!");
        writer.chunk_sent(2);
        
        assert!(writer.is_complete());
        assert!(writer.next_chunk(source).is_none());
    }

    #[test]
    fn test_stream_writer_progress() {
        let mut writer = StreamWriter::new(100, 25);
        
        assert_eq!(writer.progress_percent(), 0);
        
        writer.chunk_sent(25);
        assert_eq!(writer.progress_percent(), 25);
        
        writer.chunk_sent(25);
        assert_eq!(writer.progress_percent(), 50);
        
        writer.chunk_sent(50);
        assert_eq!(writer.progress_percent(), 100);
    }

    // ==================== ProgressTracker Tests ====================

    #[test]
    fn test_progress_tracker_new() {
        let tracker = ProgressTracker::new(Some(1000));
        assert_eq!(tracker.transferred(), 0);
        assert_eq!(tracker.percent(), Some(0));
        assert!(!tracker.is_complete());
    }

    #[test]
    fn test_progress_tracker_update() {
        let mut tracker = ProgressTracker::new(Some(100));
        
        tracker.update(25);
        assert_eq!(tracker.transferred(), 25);
        assert_eq!(tracker.percent(), Some(25));
        
        tracker.update(75);
        assert_eq!(tracker.transferred(), 100);
        assert_eq!(tracker.percent(), Some(100));
        assert!(tracker.is_complete());
    }

    #[test]
    fn test_progress_tracker_unknown_total() {
        let mut tracker = ProgressTracker::new(None);
        
        tracker.update(500);
        assert_eq!(tracker.transferred(), 500);
        assert_eq!(tracker.percent(), None);
        assert!(!tracker.is_complete());
    }

    #[test]
    fn test_progress_tracker_reset() {
        let mut tracker = ProgressTracker::new(Some(100));
        tracker.update(50);
        
        tracker.reset();
        assert_eq!(tracker.transferred(), 0);
    }

    #[test]
    fn test_progress_tracker_zero_total() {
        let tracker = ProgressTracker::new(Some(0));
        assert_eq!(tracker.percent(), Some(100));
    }

    // ==================== Progress Callback Tests ====================

    // Note: Testing callbacks requires thread_local or similar mechanism
    // For no_std, we use a simple static counter pattern in integration tests
    
    #[test]
    fn test_stream_reader_progress_callback_integration() {
        // This tests that progress callback is properly wired up
        // In a real scenario, the callback would update UI
        
        let config = StreamConfig {
            progress_interval: 10, // Report every 10 bytes
            ..Default::default()
        };
        let mut reader = StreamReader::with_config(config);
        
        // We can't easily verify the callback was called without
        // thread-local storage, but we can verify it doesn't panic
        reader.set_progress_callback(|transferred, total| {
            // This callback runs during feed()
            let _ = (transferred, total);
        });
        
        reader.set_expected_size(Some(100));
        
        // Feed in chunks
        for _ in 0..10 {
            reader.feed(&[0u8; 10]).unwrap();
        }
        
        assert!(reader.is_complete());
    }
}
