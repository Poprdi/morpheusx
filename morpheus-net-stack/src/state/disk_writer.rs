//! Streams ISO data to disk in chunks as it arrives, running in tandem with
//! the HTTP download machine. Fire-and-forget block I/O; applies backpressure
//! by refusing writes once the in-flight queue is full.

use super::StepResult;
use morpheus_block::block_traits::{BlockCompletion, BlockDriver, BlockError};

const MAX_PENDING_WRITES: usize = 16;

pub const SECTORS_PER_CHUNK: u32 = 128;

pub const CHUNK_SIZE: usize = 65536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskWriterError {
    BlockError(BlockError),
    WriteFailed {
        request_id: u32,
        status: u8,
    },
    InsufficientSpace {
        required: u64,
        available: u64,
    },
    MisalignedWrite,
    /// In-flight queue full; apply backpressure.
    QueueFull,
    InvalidState,
}

crate::impl_from!(BlockError => DiskWriterError : BlockError);

#[derive(Debug, Clone, Copy, Default)]
struct PendingWrite {
    request_id: u32,
    sector: u64,
    num_sectors: u32,
    bytes: u32,
    active: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct DiskWriterConfig {
    pub start_sector: u64,
    /// 0 = unknown; stream until the source closes.
    pub total_bytes: u64,
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

#[derive(Debug, Clone, Copy, Default)]
pub struct DiskWriterProgress {
    pub bytes_submitted: u64,
    /// Confirmed via completion.
    pub bytes_written: u64,
    /// 0 = unknown.
    pub total_bytes: u64,
    pub pending_writes: usize,
    pub completed_writes: u32,
    pub failed_writes: u32,
}

impl DiskWriterProgress {
    /// 0-100.
    pub fn percent_complete(&self) -> u8 {
        if self.total_bytes == 0 {
            return 0;
        }
        let pct = (self.bytes_written * 100) / self.total_bytes;
        pct.min(100) as u8
    }

    pub fn is_complete(&self) -> bool {
        self.total_bytes > 0 && self.bytes_written >= self.total_bytes && self.pending_writes == 0
    }

    pub fn bytes_in_flight(&self) -> u64 {
        self.bytes_submitted.saturating_sub(self.bytes_written)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriterState {
    Init,
    Ready,
    Writing,
    /// All writes submitted; draining completions.
    Flushing,
    Done,
    Failed,
}

pub(crate) struct DiskWriterState {
    state: WriterState,
    config: DiskWriterConfig,
    current_sector: u64,
    next_request_id: u32,
    pending: [PendingWrite; MAX_PENDING_WRITES],
    pending_count: usize,
    progress: DiskWriterProgress,
    error: Option<DiskWriterError>,
}

impl DiskWriterState {
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

    pub fn state(&self) -> WriterState {
        self.state
    }

    pub fn progress(&self) -> DiskWriterProgress {
        self.progress
    }

    pub fn error(&self) -> Option<DiskWriterError> {
        self.error
    }

    /// False under backpressure or when not in Ready/Writing.
    pub fn can_write(&self) -> bool {
        match self.state {
            WriterState::Ready | WriterState::Writing => self.pending_count < MAX_PENDING_WRITES,
            _ => false,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.state == WriterState::Done
    }

    pub fn has_pending(&self) -> bool {
        self.pending_count > 0
    }

    /// Validates disk capacity, then moves Init -> Ready.
    pub fn start<D: BlockDriver>(&mut self, driver: &D) -> Result<(), DiskWriterError> {
        if self.state != WriterState::Init {
            return Err(DiskWriterError::InvalidState);
        }

        if self.config.total_bytes > 0 {
            let info = driver.info();
            let required_sectors = self
                .config
                .total_bytes
                .div_ceil(self.config.sector_size as u64);
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

    /// `len` should be a multiple of sector_size; the buffer must stay valid
    /// until completion.
    pub fn write_chunk<D: BlockDriver>(
        &mut self,
        driver: &mut D,
        buffer_phys: u64,
        len: usize,
    ) -> Result<(), DiskWriterError> {
        match self.state {
            WriterState::Ready | WriterState::Writing => {},
            WriterState::Init => return Err(DiskWriterError::InvalidState),
            WriterState::Flushing => return Err(DiskWriterError::InvalidState),
            WriterState::Done => return Err(DiskWriterError::InvalidState),
            WriterState::Failed => return Err(self.error.unwrap_or(DiskWriterError::InvalidState)),
        }

        if self.pending_count >= MAX_PENDING_WRITES {
            return Err(DiskWriterError::QueueFull);
        }

        if !driver.can_submit() {
            return Err(DiskWriterError::QueueFull);
        }

        let sector_size = self.config.sector_size as usize;
        let num_sectors = len.div_ceil(sector_size) as u32;

        let slot = self.find_free_slot().ok_or(DiskWriterError::QueueFull)?;

        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        driver.submit_write(self.current_sector, buffer_phys, num_sectors, request_id)?;

        self.pending[slot] = PendingWrite {
            request_id,
            sector: self.current_sector,
            num_sectors,
            bytes: len as u32,
            active: true,
        };
        self.pending_count += 1;

        self.current_sector += num_sectors as u64;
        self.progress.bytes_submitted += len as u64;
        self.state = WriterState::Writing;

        driver.notify();

        Ok(())
    }

    /// Signal no more data; moves to Done (or Flushing if writes are in flight).
    pub fn finish(&mut self) {
        match self.state {
            WriterState::Ready | WriterState::Writing => {
                if self.pending_count == 0 {
                    self.state = WriterState::Done;
                } else {
                    self.state = WriterState::Flushing;
                }
            },
            _ => {},
        }
    }

    /// Drain write completions and advance state.
    pub fn step<D: BlockDriver>(&mut self, driver: &mut D) -> StepResult {
        while let Some(completion) = driver.poll_completion() {
            self.handle_completion(completion);
        }

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
            },
            WriterState::Done => StepResult::Done,
            WriterState::Failed => StepResult::Failed,
        }
    }

    fn handle_completion(&mut self, completion: BlockCompletion) {
        let slot = self
            .pending
            .iter()
            .position(|p| p.active && p.request_id == completion.request_id);

        let Some(slot) = slot else {
            return;
        };

        let pending = self.pending[slot];

        self.pending[slot].active = false;
        self.pending_count = self.pending_count.saturating_sub(1);

        if completion.status != 0 {
            self.progress.failed_writes += 1;
            self.error = Some(DiskWriterError::WriteFailed {
                request_id: completion.request_id,
                status: completion.status,
            });
            self.state = WriterState::Failed;
            return;
        }

        self.progress.bytes_written += pending.bytes as u64;
        self.progress.completed_writes += 1;

        if self.state == WriterState::Flushing && self.pending_count == 0 {
            self.state = WriterState::Done;
        }
    }

    fn find_free_slot(&self) -> Option<usize> {
        self.pending.iter().position(|p| !p.active)
    }

    pub fn next_sector(&self) -> u64 {
        self.current_sector
    }

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

    /// Set once Content-Length is known.
    pub fn set_total_bytes(&mut self, total: u64) {
        self.progress.total_bytes = total;
        self.config.total_bytes = total;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WriteChunk {
    pub buffer_phys: u64,
    /// CPU-side address for an optional memcpy.
    pub buffer_cpu: *const u8,
    pub len: usize,
    /// Ordering sequence.
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

/// Ring buffer decoupling HTTP receive rate from disk write rate.
pub struct ChunkQueue {
    chunks: [WriteChunk; MAX_PENDING_WRITES],
    head: usize,
    tail: usize,
    count: usize,
    next_sequence: u32,
}

impl ChunkQueue {
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

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn is_full(&self) -> bool {
        self.count >= MAX_PENDING_WRITES
    }

    pub fn len(&self) -> usize {
        self.count
    }

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

    pub fn dequeue(&mut self) -> Option<WriteChunk> {
        if self.is_empty() {
            return None;
        }

        let chunk = self.chunks[self.head];
        self.head = (self.head + 1) % MAX_PENDING_WRITES;
        self.count -= 1;

        Some(chunk)
    }

    pub fn peek(&self) -> Option<&WriteChunk> {
        if self.is_empty() {
            None
        } else {
            Some(&self.chunks[self.head])
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_queue() {
        let mut queue = ChunkQueue::new();

        assert!(queue.is_empty());
        assert!(!queue.is_full());

        let seq = queue.enqueue(0x1000, core::ptr::null(), 512);
        assert_eq!(seq, Some(0));
        assert_eq!(queue.len(), 1);

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
