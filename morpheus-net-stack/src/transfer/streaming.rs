//! Streaming download handler with progress callbacks and cancellation.

use crate::error::{NetworkError, Result};
use crate::types::ProgressCallback;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Ready,
    Receiving,
    Complete,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct StreamConfig {
    pub buffer_size: usize,
    /// Bytes between progress callbacks.
    pub progress_interval: usize,
    /// OOM guard; `None` = unbounded.
    pub max_size: Option<usize>,
    pub chunk_timeout_ms: Option<u32>,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            buffer_size: 64 * 1024,
            progress_interval: 16 * 1024,
            max_size: None,
            chunk_timeout_ms: Some(30000),
        }
    }
}

impl StreamConfig {
    /// Tuned for small files; caps at 1 MB.
    pub fn small() -> Self {
        Self {
            buffer_size: 8 * 1024,
            progress_interval: 4 * 1024,
            max_size: Some(1024 * 1024),
            chunk_timeout_ms: Some(10000),
        }
    }

    /// Tuned for ISOs etc.; no size cap.
    pub fn large() -> Self {
        Self {
            buffer_size: 256 * 1024,
            progress_interval: 1024 * 1024,
            max_size: None,
            chunk_timeout_ms: Some(60000),
        }
    }
}

/// Accumulating reader with progress callbacks.
#[derive(Debug)]
pub struct StreamReader {
    config: StreamConfig,
    state: StreamState,
    buffer: Vec<u8>,
    bytes_received: usize,
    /// From Content-Length, if present.
    expected_size: Option<usize>,
    bytes_since_progress: usize,
    progress_callback: Option<ProgressCallback>,
    cancelled: bool,
}

impl StreamReader {
    pub fn new() -> Self {
        Self::with_config(StreamConfig::default())
    }

    pub fn with_buffer_size(buffer_size: usize) -> Self {
        Self::with_config(StreamConfig {
            buffer_size,
            ..Default::default()
        })
    }

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

    pub fn set_expected_size(&mut self, size: Option<usize>) {
        self.expected_size = size;
    }

    pub fn expected_size(&self) -> Option<usize> {
        self.expected_size
    }

    pub fn set_progress_callback(&mut self, callback: ProgressCallback) {
        self.progress_callback = Some(callback);
    }

    pub fn state(&self) -> StreamState {
        self.state
    }

    pub fn is_complete(&self) -> bool {
        self.state == StreamState::Complete
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled || self.state == StreamState::Cancelled
    }

    pub fn bytes_received(&self) -> usize {
        self.bytes_received
    }

    pub fn progress_percent(&self) -> Option<u8> {
        self.expected_size.map(|total| {
            if total == 0 {
                100
            } else {
                ((self.bytes_received as u64 * 100) / total as u64) as u8
            }
        })
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.state = StreamState::Cancelled;
    }

    /// Returns bytes consumed.
    pub fn feed(&mut self, data: &[u8]) -> Result<usize> {
        if self.cancelled {
            self.state = StreamState::Cancelled;
            return Err(NetworkError::Cancelled);
        }

        if let Some(max) = self.config.max_size {
            if self.bytes_received + data.len() > max {
                self.state = StreamState::Failed;
                return Err(NetworkError::OutOfMemory);
            }
        }

        if self.state == StreamState::Ready {
            self.state = StreamState::Receiving;
        }

        self.buffer.extend_from_slice(data);
        self.bytes_received += data.len();
        self.bytes_since_progress += data.len();

        if self.bytes_since_progress >= self.config.progress_interval {
            self.report_progress();
            self.bytes_since_progress = 0;
        }

        if let Some(expected) = self.expected_size {
            if self.bytes_received >= expected {
                self.state = StreamState::Complete;
                self.report_progress();
            }
        }

        Ok(data.len())
    }

    /// Terminator for chunked-encoded bodies (no Content-Length).
    pub fn finish(&mut self) {
        if self.state == StreamState::Receiving || self.state == StreamState::Ready {
            self.state = StreamState::Complete;
            self.report_progress();
        }
    }

    pub fn fail(&mut self) {
        self.state = StreamState::Failed;
    }

    pub fn data(&self) -> &[u8] {
        &self.buffer
    }

    pub fn take_data(self) -> Vec<u8> {
        self.buffer
    }

    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.state = StreamState::Ready;
        self.bytes_received = 0;
        self.expected_size = None;
        self.bytes_since_progress = 0;
        self.cancelled = false;
    }

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

/// Chunked sender for large POST/PUT bodies.
#[derive(Debug)]
pub struct StreamWriter {
    total_size: usize,
    bytes_sent: usize,
    chunk_size: usize,
    progress_callback: Option<ProgressCallback>,
    progress_interval: usize,
    bytes_since_progress: usize,
}

impl StreamWriter {
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

    pub fn set_progress_callback(&mut self, callback: ProgressCallback) {
        self.progress_callback = Some(callback);
    }

    /// Returns `None` once all bytes are sent.
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

    pub fn chunk_sent(&mut self, bytes: usize) {
        self.bytes_sent += bytes;
        self.bytes_since_progress += bytes;

        if self.bytes_since_progress >= self.progress_interval {
            self.report_progress();
            self.bytes_since_progress = 0;
        }
    }

    pub fn is_complete(&self) -> bool {
        self.bytes_sent >= self.total_size
    }

    pub fn bytes_sent(&self) -> usize {
        self.bytes_sent
    }

    pub fn remaining(&self) -> usize {
        self.total_size.saturating_sub(self.bytes_sent)
    }

    pub fn progress_percent(&self) -> u8 {
        if self.total_size == 0 {
            100
        } else {
            ((self.bytes_sent as u64 * 100) / self.total_size as u64) as u8
        }
    }

    fn report_progress(&self) {
        if let Some(callback) = self.progress_callback {
            callback(self.bytes_sent, Some(self.total_size));
        }
    }
}

/// Standalone progress tracker; no buffering, for direct-to-sink flows.
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    total: Option<usize>,
    transferred: usize,
    callback: Option<ProgressCallback>,
    interval: usize,
    since_report: usize,
}

impl ProgressTracker {
    pub fn new(total: Option<usize>) -> Self {
        Self {
            total,
            transferred: 0,
            callback: None,
            interval: 16 * 1024,
            since_report: 0,
        }
    }

    pub fn set_callback(&mut self, callback: ProgressCallback) {
        self.callback = Some(callback);
    }

    pub fn set_interval(&mut self, interval: usize) {
        self.interval = interval;
    }

    pub fn update(&mut self, bytes: usize) {
        self.transferred += bytes;
        self.since_report += bytes;

        if self.since_report >= self.interval {
            self.report();
            self.since_report = 0;
        }
    }

    pub fn report(&self) {
        if let Some(callback) = self.callback {
            callback(self.transferred, self.total);
        }
    }

    pub fn transferred(&self) -> usize {
        self.transferred
    }

    pub fn percent(&self) -> Option<u8> {
        self.total.map(|t| {
            if t == 0 {
                100
            } else {
                ((self.transferred as u64 * 100) / t as u64) as u8
            }
        })
    }

    pub fn is_complete(&self) -> bool {
        self.total.is_some_and(|t| self.transferred >= t)
    }

    pub fn reset(&mut self) {
        self.transferred = 0;
        self.since_report = 0;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use core::cell::Cell;


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
