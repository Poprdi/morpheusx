//! Streaming writer that splits data across FAT32 chunk partitions, one data
//! file per chunk, writing directly to the block device.

use super::chunk::{ChunkInfo, ChunkSet, MAX_CHUNKS};
use super::error::IsoError;
use super::manifest::IsoManifest;
use super::{DEFAULT_CHUNK_SIZE, FAT32_MAX_FILE_SIZE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterState {
    Ready,
    Writing,
    Finalized,
    Failed,
}

/// `(bytes_written, total_bytes, current_chunk, total_chunks)`.
pub type WriteProgressFn = fn(u64, u64, usize, usize);

pub struct ChunkWriter {
    state: WriterState,
    current_chunk: usize,
    chunk_bytes_written: u64,
    total_bytes_written: u64,
    chunk_size: u64,
    total_size: u64,
    num_chunks: usize,
    /// Per-chunk (start_lba, end_lba).
    chunk_partitions: [(u64, u64); MAX_CHUNKS],
    progress_fn: Option<WriteProgressFn>,
}

impl ChunkWriter {
    /// Manifest must already have chunk partitions assigned.
    pub fn from_manifest(manifest: &IsoManifest) -> Result<Self, IsoError> {
        if manifest.chunks.count == 0 {
            return Err(IsoError::InsufficientPartitions);
        }

        let mut chunk_partitions = [(0u64, 0u64); MAX_CHUNKS];
        for i in 0..manifest.chunks.count {
            let chunk = &manifest.chunks.chunks[i];
            chunk_partitions[i] = (chunk.start_lba, chunk.end_lba);
        }

        Ok(Self {
            state: WriterState::Ready,
            current_chunk: 0,
            chunk_bytes_written: 0,
            total_bytes_written: 0,
            chunk_size: DEFAULT_CHUNK_SIZE,
            total_size: manifest.total_size,
            num_chunks: manifest.chunks.count,
            chunk_partitions,
            progress_fn: None,
        })
    }

    pub fn new(
        total_size: u64,
        chunk_size: u64,
        partitions: &[(u64, u64)],
    ) -> Result<Self, IsoError> {
        if partitions.is_empty() {
            return Err(IsoError::InsufficientPartitions);
        }
        if partitions.len() > MAX_CHUNKS {
            return Err(IsoError::IsoTooLarge);
        }

        let mut chunk_partitions = [(0u64, 0u64); MAX_CHUNKS];
        for (i, &(start, end)) in partitions.iter().enumerate() {
            chunk_partitions[i] = (start, end);
        }

        Ok(Self {
            state: WriterState::Ready,
            current_chunk: 0,
            chunk_bytes_written: 0,
            total_bytes_written: 0,
            chunk_size: chunk_size.min(FAT32_MAX_FILE_SIZE),
            total_size,
            num_chunks: partitions.len(),
            chunk_partitions,
            progress_fn: None,
        })
    }

    pub fn set_progress_fn(&mut self, f: WriteProgressFn) {
        self.progress_fn = Some(f);
    }

    pub fn state(&self) -> WriterState {
        self.state
    }

    pub fn bytes_written(&self) -> u64 {
        self.total_bytes_written
    }

    pub fn current_chunk_index(&self) -> usize {
        self.current_chunk
    }

    /// 0-100.
    pub fn progress_percent(&self) -> u8 {
        if self.total_size == 0 {
            return 100;
        }
        ((self.total_bytes_written * 100) / self.total_size) as u8
    }

    /// Returns (chunk index, offset within chunk) for a byte position.
    fn chunk_for_position(&self, position: u64) -> (usize, u64) {
        let chunk_index = (position / self.chunk_size) as usize;
        let offset_in_chunk = position % self.chunk_size;
        (chunk_index.min(self.num_chunks - 1), offset_in_chunk)
    }

    /// Splits data across chunk boundaries.
    /// `write_sector_fn(partition_start_lba, sector_offset, data)`.
    pub fn write<F>(&mut self, data: &[u8], mut write_sector_fn: F) -> Result<usize, IsoError>
    where
        F: FnMut(u64, u64, &[u8]) -> Result<(), IsoError>,
    {
        if self.state == WriterState::Finalized {
            return Err(IsoError::WriteOverflow);
        }
        if self.state == WriterState::Failed {
            return Err(IsoError::IoError);
        }

        self.state = WriterState::Writing;

        let mut bytes_written = 0usize;
        let mut remaining = data;

        while !remaining.is_empty() {
            if self.total_bytes_written >= self.total_size {
                break;
            }

            if self.chunk_bytes_written >= self.chunk_size {
                self.current_chunk += 1;
                self.chunk_bytes_written = 0;

                if self.current_chunk >= self.num_chunks {
                    self.state = WriterState::Failed;
                    return Err(IsoError::WriteOverflow);
                }
            }

            let space_in_chunk = self.chunk_size - self.chunk_bytes_written;
            let space_to_end = self.total_size - self.total_bytes_written;
            let write_size = (remaining.len() as u64)
                .min(space_in_chunk)
                .min(space_to_end) as usize;

            if write_size == 0 {
                break;
            }

            let (part_start_lba, _part_end_lba) = self.chunk_partitions[self.current_chunk];

            // Data area starts at sector 8192, past 32 reserved + FAT tables (~4 GB partition).
            const DATA_START_SECTOR: u64 = 8192;
            let sector_offset = DATA_START_SECTOR + (self.chunk_bytes_written / 512);

            write_sector_fn(part_start_lba, sector_offset, &remaining[..write_size])?;

            self.chunk_bytes_written += write_size as u64;
            self.total_bytes_written += write_size as u64;
            bytes_written += write_size;
            remaining = &remaining[write_size..];

            if let Some(f) = self.progress_fn {
                f(
                    self.total_bytes_written,
                    self.total_size,
                    self.current_chunk,
                    self.num_chunks,
                );
            }
        }

        Ok(bytes_written)
    }

    /// Call after all data is written; rebuilds chunk metadata with final sizes.
    pub fn finalize(&mut self) -> Result<ChunkSet, IsoError> {
        if self.state == WriterState::Finalized {
            return Err(IsoError::NotSupported);
        }

        let mut chunks = ChunkSet::new();
        chunks.total_size = self.total_size;
        chunks.bytes_written = self.total_bytes_written;

        let mut remaining_bytes = self.total_bytes_written;
        for i in 0..self.num_chunks {
            let (start_lba, end_lba) = self.chunk_partitions[i];
            let chunk_data_size = remaining_bytes.min(self.chunk_size);

            let mut info = ChunkInfo::new([0u8; 16], start_lba, end_lba, i as u8);
            info.data_size = chunk_data_size;
            info.written = chunk_data_size > 0;

            chunks.add_chunk(info);

            if chunk_data_size >= remaining_bytes {
                break;
            }
            remaining_bytes -= chunk_data_size;
        }

        self.state = WriterState::Finalized;
        Ok(chunks)
    }

    /// Reset for a new ISO, reusing allocated partitions.
    pub fn reset(&mut self, new_total_size: u64) {
        self.state = WriterState::Ready;
        self.current_chunk = 0;
        self.chunk_bytes_written = 0;
        self.total_bytes_written = 0;
        self.total_size = new_total_size;
    }
}

/// Returns (chunk_index, data_size) pairs for the ISO.
pub fn calculate_chunk_layout(iso_size: u64, chunk_size: u64) -> [(u8, u64); MAX_CHUNKS] {
    let mut layout = [(0u8, 0u64); MAX_CHUNKS];
    let effective_chunk_size = chunk_size.min(FAT32_MAX_FILE_SIZE);

    let mut remaining = iso_size;
    let mut index = 0u8;

    while remaining > 0 && (index as usize) < MAX_CHUNKS {
        let this_chunk = remaining.min(effective_chunk_size);
        layout[index as usize] = (index, this_chunk);
        remaining -= this_chunk;
        index += 1;
    }

    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_layout_single() {
        let layout = calculate_chunk_layout(1_000_000_000, DEFAULT_CHUNK_SIZE);
        assert_eq!(layout[0], (0, 1_000_000_000));
        assert_eq!(layout[1], (0, 0));
    }

    #[test]
    fn test_chunk_layout_multiple() {
        let chunk_size = 1_000_000_000;
        let layout = calculate_chunk_layout(2_500_000_000, chunk_size);
        assert_eq!(layout[0], (0, 1_000_000_000));
        assert_eq!(layout[1], (1, 1_000_000_000));
        assert_eq!(layout[2], (2, 500_000_000));
        assert_eq!(layout[3], (0, 0));
    }

    #[test]
    fn test_writer_progress() {
        let partitions = [(100, 9000000), (9000001, 18000000)];
        let writer = ChunkWriter::new(5_000_000_000, 4_000_000_000, &partitions).unwrap();

        assert_eq!(writer.progress_percent(), 0);
        assert_eq!(writer.current_chunk_index(), 0);
        assert_eq!(writer.state(), WriterState::Ready);
    }
}
