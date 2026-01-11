//! Block I/O Adapter for Chunked ISO Storage
//!
//! This module provides a `BlockIo` implementation that presents chunked
//! ISO storage as a single contiguous block device. This allows the iso9660
//! crate to read from multi-partition ISOs transparently.
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_core::iso::{ChunkReader, IsoReadContext};
//! use morpheus_core::iso::adapter::ChunkedBlockIo;
//!
//! // Create adapter from read context
//! let ctx = storage_manager.get_read_context(iso_index)?;
//! let mut adapter = ChunkedBlockIo::new(ctx, |lba, buf| {
//!     // Read from actual disk
//!     disk_block_io.read_blocks(Lba(lba), buf)
//! });
//!
//! // Now use with iso9660
//! let volume = iso9660::mount(&mut adapter, 0)?;
//! let file = iso9660::find_file(&mut adapter, &volume, "/boot/vmlinuz")?;
//! ```

use super::error::IsoError;
use super::reader::IsoReadContext;

/// Sector size (standard)
const SECTOR_SIZE: usize = 512;

/// Block I/O adapter for chunked ISO storage
///
/// Implements a virtual block device over chunked partitions.
pub struct ChunkedBlockIo<F>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), IsoError>,
{
    /// ISO read context (chunk locations)
    ctx: IsoReadContext,
    /// Callback to read from underlying disk
    read_fn: F,
    /// Cached block size
    block_size: u32,
    /// Total sectors in virtual device
    total_sectors: u64,
}

impl<F> ChunkedBlockIo<F>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), IsoError>,
{
    /// Create a new chunked block I/O adapter
    ///
    /// # Arguments
    /// * `ctx` - ISO read context with chunk partition info
    /// * `read_fn` - Callback to read from disk: `fn(lba, buffer) -> Result`
    pub fn new(ctx: IsoReadContext, read_fn: F) -> Self {
        let total_sectors = ctx.total_size / SECTOR_SIZE as u64;

        Self {
            ctx,
            read_fn,
            block_size: SECTOR_SIZE as u32,
            total_sectors,
        }
    }

    /// Get total size in bytes
    pub fn total_size(&self) -> u64 {
        self.ctx.total_size
    }

    /// Get total number of sectors
    pub fn total_sectors(&self) -> u64 {
        self.total_sectors
    }

    /// Find which chunk contains a given virtual LBA
    fn find_chunk_for_lba(&self, virtual_lba: u64) -> Option<(usize, u64)> {
        let byte_offset = virtual_lba * SECTOR_SIZE as u64;
        let mut cumulative = 0u64;

        for i in 0..self.ctx.num_chunks {
            let chunk_size = self.ctx.chunk_sizes[i];
            if byte_offset < cumulative + chunk_size {
                // Found the chunk
                let offset_in_chunk = byte_offset - cumulative;
                return Some((i, offset_in_chunk));
            }
            cumulative += chunk_size;
        }
        None
    }

    /// Read a single sector
    pub fn read_sector(&mut self, virtual_lba: u64, buffer: &mut [u8]) -> Result<(), IsoError> {
        if buffer.len() < SECTOR_SIZE {
            return Err(IsoError::IoError);
        }

        if virtual_lba >= self.total_sectors {
            return Err(IsoError::ReadOverflow);
        }

        // Find which chunk this LBA belongs to
        let (chunk_idx, offset_in_chunk) = self
            .find_chunk_for_lba(virtual_lba)
            .ok_or(IsoError::ChunkNotFound)?;

        // Calculate physical LBA
        // Data starts at sector 8192 in each chunk partition (matching writer.rs)
        const DATA_START_SECTOR: u64 = 8192;
        let (part_start, _part_end) = self.ctx.chunk_lbas[chunk_idx];
        let sector_in_chunk = offset_in_chunk / SECTOR_SIZE as u64;
        let physical_lba = part_start + DATA_START_SECTOR + sector_in_chunk;

        // Read from disk
        (self.read_fn)(physical_lba, &mut buffer[..SECTOR_SIZE])
    }

    /// Read multiple sectors
    pub fn read_sectors(&mut self, start_lba: u64, buffer: &mut [u8]) -> Result<usize, IsoError> {
        let sector_count = buffer.len() / SECTOR_SIZE;
        let mut bytes_read = 0;

        for i in 0..sector_count {
            let lba = start_lba + i as u64;
            if lba >= self.total_sectors {
                break;
            }

            let offset = i * SECTOR_SIZE;
            self.read_sector(lba, &mut buffer[offset..offset + SECTOR_SIZE])?;
            bytes_read += SECTOR_SIZE;
        }

        Ok(bytes_read)
    }
}

/// Trait for block I/O operations (matches gpt_disk_io::BlockIo pattern)
pub trait VirtualBlockIo {
    /// Read blocks starting at the given LBA
    fn read_blocks(&mut self, lba: u64, buffer: &mut [u8]) -> Result<(), IsoError>;

    /// Get block size
    fn block_size(&self) -> u32;

    /// Get total number of blocks
    fn num_blocks(&self) -> u64;
}

impl<F> VirtualBlockIo for ChunkedBlockIo<F>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), IsoError>,
{
    fn read_blocks(&mut self, lba: u64, buffer: &mut [u8]) -> Result<(), IsoError> {
        self.read_sectors(lba, buffer)?;
        Ok(())
    }

    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn num_blocks(&self) -> u64 {
        self.total_sectors
    }
}

/// Simpler read interface for when you just need byte-level access
pub struct ChunkedReader<F>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), IsoError>,
{
    block_io: ChunkedBlockIo<F>,
    /// Current position
    position: u64,
    /// Sector buffer for unaligned reads
    sector_buf: [u8; SECTOR_SIZE],
}

impl<F> ChunkedReader<F>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), IsoError>,
{
    /// Create a new chunked reader
    pub fn new(ctx: IsoReadContext, read_fn: F) -> Self {
        Self {
            block_io: ChunkedBlockIo::new(ctx, read_fn),
            position: 0,
            sector_buf: [0u8; SECTOR_SIZE],
        }
    }

    /// Get total size
    pub fn total_size(&self) -> u64 {
        self.block_io.total_size()
    }

    /// Get current position
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Seek to position
    pub fn seek(&mut self, pos: u64) -> Result<(), IsoError> {
        if pos > self.block_io.total_size() {
            return Err(IsoError::ReadOverflow);
        }
        self.position = pos;
        Ok(())
    }

    /// Read bytes at current position
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, IsoError> {
        let total_size = self.block_io.total_size();
        if self.position >= total_size {
            return Ok(0);
        }

        let available = (total_size - self.position) as usize;
        let to_read = buffer.len().min(available);
        let mut bytes_read = 0;

        while bytes_read < to_read {
            let current_pos = self.position + bytes_read as u64;
            let sector_lba = current_pos / SECTOR_SIZE as u64;
            let offset_in_sector = (current_pos % SECTOR_SIZE as u64) as usize;

            // Read the sector
            self.block_io
                .read_sector(sector_lba, &mut self.sector_buf)?;

            // Copy relevant portion
            let remaining = to_read - bytes_read;
            let available_in_sector = SECTOR_SIZE - offset_in_sector;
            let copy_len = remaining.min(available_in_sector);

            buffer[bytes_read..bytes_read + copy_len]
                .copy_from_slice(&self.sector_buf[offset_in_sector..offset_in_sector + copy_len]);

            bytes_read += copy_len;
        }

        self.position += bytes_read as u64;
        Ok(bytes_read)
    }

    /// Read exact number of bytes (error if not available)
    pub fn read_exact(&mut self, buffer: &mut [u8]) -> Result<(), IsoError> {
        let n = self.read(buffer)?;
        if n != buffer.len() {
            Err(IsoError::ReadOverflow)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iso::chunk::MAX_CHUNKS;

    fn make_test_context() -> IsoReadContext {
        let mut ctx = IsoReadContext {
            chunk_lbas: [(0, 0); MAX_CHUNKS],
            chunk_sizes: [0; MAX_CHUNKS],
            num_chunks: 2,
            total_size: 2_000_000_000, // 2GB
        };

        // Chunk 0: 1GB at LBA 10000
        ctx.chunk_lbas[0] = (10000, 2000000);
        ctx.chunk_sizes[0] = 1_000_000_000;

        // Chunk 1: 1GB at LBA 3000000
        ctx.chunk_lbas[1] = (3000000, 5000000);
        ctx.chunk_sizes[1] = 1_000_000_000;

        ctx
    }

    #[test]
    fn test_find_chunk() {
        let ctx = make_test_context();
        let adapter = ChunkedBlockIo::new(ctx, |_, _| Ok(()));

        // LBA 0 should be in chunk 0
        assert_eq!(adapter.find_chunk_for_lba(0), Some((0, 0)));

        // LBA near end of chunk 0
        let sectors_in_1gb = 1_000_000_000 / 512;
        assert_eq!(
            adapter.find_chunk_for_lba(sectors_in_1gb - 1),
            Some((0, (sectors_in_1gb - 1) * 512))
        );

        // First LBA of chunk 1
        assert_eq!(adapter.find_chunk_for_lba(sectors_in_1gb), Some((1, 0)));
    }

    #[test]
    fn test_total_sectors() {
        let ctx = make_test_context();
        let adapter = ChunkedBlockIo::new(ctx, |_, _| Ok(()));

        let expected = 2_000_000_000 / 512;
        assert_eq!(adapter.total_sectors(), expected);
    }
}
