//! ISO Streaming Disk Writer State Machine.
//!
//! Writes ISO data to disk in chunks as it arrives from the network.
//! Operates in tandem with HTTP download state machine.
//!
//! # Design
//!
//! - Non-blocking: Uses fire-and-forget block I/O
//! - Streaming: Writes arrive in chunks, queued to disk
//! - Backpressure: Pauses HTTP when write queue is full
//! - Progress: Tracks bytes written and completion status
//!
//! # Architecture
//!
//! ```text
//! HTTP Download ──┬── Data Chunk ──▶ DiskWriter ──▶ VirtIO-blk
//!                 │
//!                 └── Backpressure ◀────────────────┘
//! ```
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5, §8

use super::StepResult;
use crate::driver::block_traits::{BlockCompletion, BlockDriver, BlockError};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum number of in-flight write requests.
const MAX_PENDING_WRITES: usize = 16;

/// Sectors per write chunk (128 sectors = 64KB at 512B/sector).
pub const SECTORS_PER_CHUNK: u32 = 128;

/// Chunk size in bytes (64KB).
pub const CHUNK_SIZE: usize = 65536;

// ═══════════════════════════════════════════════════════════════════════════
// ERROR TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Disk writer errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskWriterError {
    /// Block device error.
    BlockError(BlockError),
    /// Write failed (device returned error status).
    WriteFailed { request_id: u32, status: u8 },
    /// Not enough disk space.
    InsufficientSpace { required: u64, available: u64 },
    /// Invalid sector alignment.
    MisalignedWrite,
    /// Write queue is full (backpressure).
    QueueFull,
    /// Writer is not in writable state.
    InvalidState,
}

impl From<BlockError> for DiskWriterError {
    fn from(e: BlockError) -> Self {
        DiskWriterError::BlockError(e)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// WRITE REQUEST TRACKING
// ═══════════════════════════════════════════════════════════════════════════

/// Pending write request.
#[derive(Debug, Clone, Copy)]
struct PendingWrite {
    /// Request ID (unique per write).
    request_id: u32,
    /// Starting sector.
    sector: u64,
    /// Number of sectors.
    num_sectors: u32,
    /// Number of bytes in this write.
    bytes: u32,
    /// Whether this request is active.
    active: bool,
}

impl Default for PendingWrite {
    fn default() -> Self {
        Self {
            request_id: 0,
            sector: 0,
            num_sectors: 0,
            bytes: 0,
            active: false,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// WRITER CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// Disk writer configuration.
#[derive(Debug, Clone, Copy)]
pub struct DiskWriterConfig {
    /// Starting sector for ISO data.
    pub start_sector: u64,
    /// Total size to write in bytes (0 = unknown, stream until done).
    pub total_bytes: u64,
    /// Sector size (typically 512).
    pub sector_size: u32,
}

impl Default for DiskWriterConfig {
    fn default() -> Self {
        Self {
            start_sector: 0,
            total_bytes: 0,
            sector_size: 512,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// WRITER PROGRESS
// ═══════════════════════════════════════════════════════════════════════════

/// Disk write progress.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiskWriterProgress {
    /// Bytes submitted for writing.
    pub bytes_submitted: u64,
    /// Bytes confirmed written (completion received).
    pub bytes_written: u64,
    /// Total bytes expected (0 = unknown).
    pub total_bytes: u64,
    /// Number of pending (unconfirmed) writes.
    pub pending_writes: usize,
    /// Number of completed writes.
    pub completed_writes: u32,
    /// Number of failed writes.
    pub failed_writes: u32,
}

impl DiskWriterProgress {
    /// Get write progress as percentage (0-100).
    pub fn percent_complete(&self) -> u8 {
        if self.total_bytes == 0 {
            return 0;
        }
        let pct = (self.bytes_written * 100) / self.total_bytes;
        pct.min(100) as u8
    }

    /// Check if all data has been written.
    pub fn is_complete(&self) -> bool {
        self.total_bytes > 0 && self.bytes_written >= self.total_bytes && self.pending_writes == 0
    }

    /// Get bytes still in flight.
    pub fn bytes_in_flight(&self) -> u64 {
        self.bytes_submitted.saturating_sub(self.bytes_written)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STATE MACHINE
// ═══════════════════════════════════════════════════════════════════════════

/// Disk writer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterState {
    /// Initial state, not yet started.
    Init,
    /// Ready to accept writes.
    Ready,
    /// Writing data (may have pending writes).
    Writing,
    /// All writes submitted, waiting for completions.
    Flushing,
    /// All writes completed successfully.
    Done,
    /// Write failed.
    Failed,
}

/// ISO streaming disk writer state machine.
///
/// Manages non-blocking writes of ISO data to disk.
///
/// # Usage
///
/// ```ignore
/// let mut writer = DiskWriterState::new(config);
/// writer.start(&mut block_driver)?;
///
/// // Write chunks as they arrive from HTTP
/// while let Some(chunk) = http_download.next_chunk() {
///     match writer.write_chunk(&mut block_driver, chunk_phys, chunk.len()) {
///         Ok(()) => {},
///         Err(DiskWriterError::QueueFull) => {
///             // Backpressure: wait for completions
///         }
///         Err(e) => return Err(e),
///     }
/// }
///
/// // Flush and wait for all completions
/// writer.finish();
/// while writer.step(&mut block_driver).is_pending() {
///     // Poll completions
/// }
/// ```
pub struct DiskWriterState {
    /// Current state.
    state: WriterState,
    /// Configuration.
    config: DiskWriterConfig,
    /// Current sector for next write.
    current_sector: u64,
    /// Next request ID.
    next_request_id: u32,
    /// Pending writes.
    pending: [PendingWrite; MAX_PENDING_WRITES],
    /// Number of active pending writes.
    pending_count: usize,
    /// Progress tracking.
    progress: DiskWriterProgress,
    /// Error (if any).
    error: Option<DiskWriterError>,
}

impl DiskWriterState {
    /// Create new disk writer with configuration.
    pub fn new(config: DiskWriterConfig) -> Self {
        Self {
            state: WriterState::Init,
            config,
            current_sector: config.start_sector,
            next_request_id: 1,
            pending: [PendingWrite::default(); MAX_PENDING_WRITES],
            pending_count: 0,
            progress: DiskWriterProgress {
                total_bytes: config.total_bytes,
                ..Default::default()
            },
            error: None,
        }
    }

    /// Get current state.
    pub fn state(&self) -> WriterState {
        self.state
    }

    /// Get write progress.
    pub fn progress(&self) -> DiskWriterProgress {
        self.progress
    }

    /// Get error (if in Failed state).
    pub fn error(&self) -> Option<DiskWriterError> {
        self.error
    }

    /// Check if writer can accept more writes.
    ///
    /// Returns false if:
    /// - Write queue is full (backpressure)
    /// - Writer is not in Ready/Writing state
    pub fn can_write(&self) -> bool {
        match self.state {
            WriterState::Ready | WriterState::Writing => self.pending_count < MAX_PENDING_WRITES,
            _ => false,
        }
    }

    /// Check if all writes are complete.
    pub fn is_complete(&self) -> bool {
        self.state == WriterState::Done
    }

    /// Check if writer has pending writes.
    pub fn has_pending(&self) -> bool {
        self.pending_count > 0
    }

    // ═══════════════════════════════════════════════════════════════════════
    // STATE TRANSITIONS
    // ═══════════════════════════════════════════════════════════════════════

    /// Start the writer.
    ///
    /// Validates that disk has sufficient space.
    pub fn start<D: BlockDriver>(&mut self, driver: &D) -> Result<(), DiskWriterError> {
        if self.state != WriterState::Init {
            return Err(DiskWriterError::InvalidState);
        }

        // Check disk capacity if total size known
        if self.config.total_bytes > 0 {
            let info = driver.info();
            let required_sectors = (self.config.total_bytes + self.config.sector_size as u64 - 1)
                / self.config.sector_size as u64;
            let available_sectors = info.total_sectors.saturating_sub(self.config.start_sector);

            if required_sectors > available_sectors {
                return Err(DiskWriterError::InsufficientSpace {
                    required: required_sectors * self.config.sector_size as u64,
                    available: available_sectors * self.config.sector_size as u64,
                });
            }
        }

        self.state = WriterState::Ready;
        Ok(())
    }

    /// Write a chunk of data to disk.
    ///
    /// # Arguments
    /// - `driver`: Block driver to use
    /// - `buffer_phys`: Physical address of data buffer
    /// - `len`: Length of data in bytes
    ///
    /// # Returns
    /// - `Ok(())`: Write submitted
    /// - `Err(QueueFull)`: Backpressure, try again after polling completions
    /// - `Err(...)`: Write failed
    ///
    /// # Contract
    /// - `len` should be multiple of sector_size for best performance
    /// - Buffer must remain valid until completion
    pub fn write_chunk<D: BlockDriver>(
        &mut self,
        driver: &mut D,
        buffer_phys: u64,
        len: usize,
    ) -> Result<(), DiskWriterError> {
        // State check
        match self.state {
            WriterState::Ready | WriterState::Writing => {}
            WriterState::Init => return Err(DiskWriterError::InvalidState),
            WriterState::Flushing => return Err(DiskWriterError::InvalidState),
            WriterState::Done => return Err(DiskWriterError::InvalidState),
            WriterState::Failed => return Err(self.error.unwrap_or(DiskWriterError::InvalidState)),
        }

        // Backpressure check
        if self.pending_count >= MAX_PENDING_WRITES {
            return Err(DiskWriterError::QueueFull);
        }

        // Driver queue check
        if !driver.can_submit() {
            return Err(DiskWriterError::QueueFull);
        }

        // Calculate sectors
        let sector_size = self.config.sector_size as usize;
        let num_sectors = ((len + sector_size - 1) / sector_size) as u32;

        // Find free pending slot
        let slot = self.find_free_slot().ok_or(DiskWriterError::QueueFull)?;

        // Allocate request ID
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        // Submit write to driver
        driver.submit_write(self.current_sector, buffer_phys, num_sectors, request_id)?;

        // Track pending write
        self.pending[slot] = PendingWrite {
            request_id,
            sector: self.current_sector,
            num_sectors,
            bytes: len as u32,
            active: true,
        };
        self.pending_count += 1;

        // Update state
        self.current_sector += num_sectors as u64;
        self.progress.bytes_submitted += len as u64;
        self.state = WriterState::Writing;

        // Notify driver
        driver.notify();

        Ok(())
    }

    /// Mark writing as finished (no more data coming).
    ///
    /// Transitions to Flushing state to wait for pending completions.
    pub fn finish(&mut self) {
        match self.state {
            WriterState::Ready | WriterState::Writing => {
                if self.pending_count == 0 {
                    self.state = WriterState::Done;
                } else {
                    self.state = WriterState::Flushing;
                }
            }
            _ => {}
        }
    }

    /// Step the state machine (poll completions).
    ///
    /// Should be called regularly to process write completions.
    ///
    /// # Returns
    /// - `Pending`: More completions expected
    /// - `Done`: All writes completed
    /// - `Failed`: A write failed
    pub fn step<D: BlockDriver>(&mut self, driver: &mut D) -> StepResult {
        // Poll completions
        while let Some(completion) = driver.poll_completion() {
            self.handle_completion(completion);
        }

        // Check state
        match self.state {
            WriterState::Init => StepResult::Pending,
            WriterState::Ready => StepResult::Pending,
            WriterState::Writing => StepResult::Pending,
            WriterState::Flushing => {
                if self.pending_count == 0 {
                    self.state = WriterState::Done;
                    StepResult::Done
                } else {
                    StepResult::Pending
                }
            }
            WriterState::Done => StepResult::Done,
            WriterState::Failed => StepResult::Failed,
        }
    }

    /// Process a write completion.
    fn handle_completion(&mut self, completion: BlockCompletion) {
        // Find matching pending write
        let slot = self
            .pending
            .iter()
            .position(|p| p.active && p.request_id == completion.request_id);

        let Some(slot) = slot else {
            // Completion for unknown request (shouldn't happen)
            return;
        };

        let pending = self.pending[slot];

        // Mark slot as free
        self.pending[slot].active = false;
        self.pending_count = self.pending_count.saturating_sub(1);

        // Check status
        if completion.status != 0 {
            // Write failed
            self.progress.failed_writes += 1;
            self.error = Some(DiskWriterError::WriteFailed {
                request_id: completion.request_id,
                status: completion.status,
            });
            self.state = WriterState::Failed;
            return;
        }

        // Success
        self.progress.bytes_written += pending.bytes as u64;
        self.progress.completed_writes += 1;

        // Check if done
        if self.state == WriterState::Flushing && self.pending_count == 0 {
            self.state = WriterState::Done;
        }
    }

    /// Find a free pending slot.
    fn find_free_slot(&self) -> Option<usize> {
        self.pending.iter().position(|p| !p.active)
    }

    // ═══════════════════════════════════════════════════════════════════════
    // HELPERS
    // ═══════════════════════════════════════════════════════════════════════

    /// Get the next sector to write to.
    pub fn next_sector(&self) -> u64 {
        self.current_sector
    }

    /// Get remaining bytes to write (if total known).
    pub fn remaining_bytes(&self) -> Option<u64> {
        if self.progress.total_bytes > 0 {
            Some(
                self.progress
                    .total_bytes
                    .saturating_sub(self.progress.bytes_submitted),
            )
        } else {
            None
        }
    }

    /// Update total bytes (e.g., after HTTP Content-Length received).
    pub fn set_total_bytes(&mut self, total: u64) {
        self.progress.total_bytes = total;
        self.config.total_bytes = total;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CHUNK BUFFER
// ═══════════════════════════════════════════════════════════════════════════

/// Write chunk descriptor for queuing.
///
/// Used to track chunks ready for writing.
#[derive(Debug, Clone, Copy)]
pub struct WriteChunk {
    /// Physical address of buffer.
    pub buffer_phys: u64,
    /// CPU address of buffer (for memcpy if needed).
    pub buffer_cpu: *const u8,
    /// Length of valid data.
    pub len: usize,
    /// Sequence number (for ordering).
    pub sequence: u32,
}

impl Default for WriteChunk {
    fn default() -> Self {
        Self {
            buffer_phys: 0,
            buffer_cpu: core::ptr::null(),
            len: 0,
            sequence: 0,
        }
    }
}

/// Chunk queue for buffering writes.
///
/// Used to decouple HTTP receive rate from disk write rate.
pub struct ChunkQueue {
    /// Queued chunks.
    chunks: [WriteChunk; MAX_PENDING_WRITES],
    /// Head index (next to dequeue).
    head: usize,
    /// Tail index (next slot to enqueue).
    tail: usize,
    /// Number of queued chunks.
    count: usize,
    /// Next sequence number.
    next_sequence: u32,
}

impl ChunkQueue {
    /// Create empty chunk queue.
    pub const fn new() -> Self {
        Self {
            chunks: [WriteChunk {
                buffer_phys: 0,
                buffer_cpu: core::ptr::null(),
                len: 0,
                sequence: 0,
            }; MAX_PENDING_WRITES],
            head: 0,
            tail: 0,
            count: 0,
            next_sequence: 0,
        }
    }

    /// Check if queue is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Check if queue is full.
    pub fn is_full(&self) -> bool {
        self.count >= MAX_PENDING_WRITES
    }

    /// Get number of queued chunks.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Enqueue a chunk.
    pub fn enqueue(&mut self, buffer_phys: u64, buffer_cpu: *const u8, len: usize) -> Option<u32> {
        if self.is_full() {
            return None;
        }

        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);

        self.chunks[self.tail] = WriteChunk {
            buffer_phys,
            buffer_cpu,
            len,
            sequence,
        };

        self.tail = (self.tail + 1) % MAX_PENDING_WRITES;
        self.count += 1;

        Some(sequence)
    }

    /// Dequeue a chunk.
    pub fn dequeue(&mut self) -> Option<WriteChunk> {
        if self.is_empty() {
            return None;
        }

        let chunk = self.chunks[self.head];
        self.head = (self.head + 1) % MAX_PENDING_WRITES;
        self.count -= 1;

        Some(chunk)
    }

    /// Peek at front chunk without removing.
    pub fn peek(&self) -> Option<&WriteChunk> {
        if self.is_empty() {
            None
        } else {
            Some(&self.chunks[self.head])
        }
    }

    /// Clear the queue.
    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }
}

impl Default for ChunkQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_queue() {
        let mut queue = ChunkQueue::new();

        assert!(queue.is_empty());
        assert!(!queue.is_full());

        // Enqueue
        let seq = queue.enqueue(0x1000, core::ptr::null(), 512);
        assert_eq!(seq, Some(0));
        assert_eq!(queue.len(), 1);

        // Dequeue
        let chunk = queue.dequeue();
        assert!(chunk.is_some());
        let chunk = chunk.unwrap();
        assert_eq!(chunk.buffer_phys, 0x1000);
        assert_eq!(chunk.len, 512);
        assert_eq!(chunk.sequence, 0);

        assert!(queue.is_empty());
    }

    #[test]
    fn test_progress_percent() {
        let mut progress = DiskWriterProgress::default();
        progress.total_bytes = 1000;

        progress.bytes_written = 0;
        assert_eq!(progress.percent_complete(), 0);

        progress.bytes_written = 500;
        assert_eq!(progress.percent_complete(), 50);

        progress.bytes_written = 1000;
        assert_eq!(progress.percent_complete(), 100);
    }
}
