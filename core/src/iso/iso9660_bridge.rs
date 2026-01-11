//! ISO9660 Integration
//!
//! Bridge between chunked ISO storage and the iso9660 crate.
//! Provides a `BlockIo` implementation that iso9660 can use directly.
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_core::iso::{IsoStorageManager, IsoBlockIoAdapter};
//! use iso9660::{mount, find_file, find_boot_image};
//!
//! // Get ISO read context from storage manager
//! let ctx = storage_manager.get_read_context(iso_index)?;
//!
//! // Create adapter with disk read function
//! let mut adapter = IsoBlockIoAdapter::new(ctx, &mut disk_block_io);
//!
//! // Use with iso9660
//! let volume = mount(&mut adapter, 0)?;
//! let boot = find_boot_image(&mut adapter, &volume)?;
//! let kernel_file = find_file(&mut adapter, &volume, "/boot/vmlinuz")?;
//! ```

use super::reader::IsoReadContext;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// ISO9660 sector size
const SECTOR_SIZE: usize = 2048;

/// Standard block size (for LBA calculations)
const BLOCK_SIZE: usize = 512;

/// Block I/O adapter that implements gpt_disk_io::BlockIo
///
/// This allows iso9660 to read from chunked ISO storage transparently.
/// The adapter translates virtual sector addresses to physical disk locations.
pub struct IsoBlockIoAdapter<'a, B: BlockIo> {
    /// ISO read context (chunk partition info)
    ctx: IsoReadContext,
    /// Underlying block device
    block_io: &'a mut B,
    /// Block size (512 for compatibility)
    block_size: u32,
}

impl<'a, B: BlockIo> IsoBlockIoAdapter<'a, B> {
    /// Create a new adapter
    ///
    /// # Arguments
    /// * `ctx` - ISO read context from IsoStorageManager
    /// * `block_io` - Underlying disk block device
    pub fn new(ctx: IsoReadContext, block_io: &'a mut B) -> Self {
        Self {
            ctx,
            block_io,
            block_size: BLOCK_SIZE as u32,
        }
    }

    /// Get total size in bytes
    pub fn total_size(&self) -> u64 {
        self.ctx.total_size
    }

    /// Get total number of 512-byte blocks
    pub fn total_blocks(&self) -> u64 {
        self.ctx.total_size / BLOCK_SIZE as u64
    }

    /// Find which chunk contains a given byte offset
    fn find_chunk_for_offset(&self, byte_offset: u64) -> Option<(usize, u64)> {
        let mut cumulative = 0u64;

        for i in 0..self.ctx.num_chunks {
            let chunk_size = self.ctx.chunk_sizes[i];
            if byte_offset < cumulative + chunk_size {
                let offset_in_chunk = byte_offset - cumulative;
                return Some((i, offset_in_chunk));
            }
            cumulative += chunk_size;
        }
        None
    }

    /// Translate virtual LBA to physical disk LBA
    fn translate_lba(&self, virtual_lba: u64) -> Option<u64> {
        let byte_offset = virtual_lba * BLOCK_SIZE as u64;
        let (chunk_idx, offset_in_chunk) = self.find_chunk_for_offset(byte_offset)?;

        // Data starts at sector 8192 in each chunk partition (FAT32 data area)
        const DATA_START_SECTOR: u64 = 8192;
        let (part_start, _) = self.ctx.chunk_lbas[chunk_idx];
        let sector_in_chunk = offset_in_chunk / BLOCK_SIZE as u64;
        let physical_lba = part_start + DATA_START_SECTOR + sector_in_chunk;

        Some(physical_lba)
    }
}

impl<'a, B: BlockIo> BlockIo for IsoBlockIoAdapter<'a, B> {
    type Error = B::Error;

    fn block_size(&self) -> gpt_disk_types::BlockSize {
        gpt_disk_types::BlockSize::new(self.block_size).unwrap()
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.total_blocks())
    }

    fn read_blocks(&mut self, start_lba: Lba, buffer: &mut [u8]) -> Result<(), Self::Error> {
        let num_blocks = buffer.len() / BLOCK_SIZE;

        for i in 0..num_blocks {
            let virtual_lba = start_lba.0 + i as u64;

            // Check bounds
            if virtual_lba >= self.total_blocks() {
                // Read beyond EOF - fill with zeros (common for ISO padding)
                let offset = i * BLOCK_SIZE;
                buffer[offset..offset + BLOCK_SIZE].fill(0);
                continue;
            }

            // Translate to physical LBA
            let physical_lba = match self.translate_lba(virtual_lba) {
                Some(lba) => lba,
                None => {
                    // Chunk not found - fill with zeros
                    let offset = i * BLOCK_SIZE;
                    buffer[offset..offset + BLOCK_SIZE].fill(0);
                    continue;
                }
            };

            // Read from disk
            let offset = i * BLOCK_SIZE;
            self.block_io
                .read_blocks(Lba(physical_lba), &mut buffer[offset..offset + BLOCK_SIZE])?;
        }

        Ok(())
    }

    fn write_blocks(&mut self, _start_lba: Lba, _buffer: &[u8]) -> Result<(), Self::Error> {
        // ISO storage is read-only for booting
        // Return success but don't actually write (iso9660 shouldn't write anyway)
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.block_io.flush()
    }
}

/// High-level helper to boot from a chunked ISO
///
/// Combines ISO storage access with iso9660 parsing in a single interface.
pub struct ChunkedIso<'a, B: BlockIo> {
    adapter: IsoBlockIoAdapter<'a, B>,
}

impl<'a, B: BlockIo> ChunkedIso<'a, B> {
    /// Create a new chunked ISO accessor
    pub fn new(ctx: IsoReadContext, block_io: &'a mut B) -> Self {
        Self {
            adapter: IsoBlockIoAdapter::new(ctx, block_io),
        }
    }

    /// Get mutable reference to the adapter for iso9660 operations
    pub fn block_io(&mut self) -> &mut IsoBlockIoAdapter<'a, B> {
        &mut self.adapter
    }

    /// Get ISO total size
    pub fn total_size(&self) -> u64 {
        self.adapter.total_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt;

    // Mock error type that implements Display
    #[derive(Debug)]
    struct MockError;

    impl fmt::Display for MockError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "MockError")
        }
    }

    // Mock BlockIo for testing
    struct MockBlockIo {
        data: [u8; 4096],
    }

    impl BlockIo for MockBlockIo {
        type Error = MockError;

        fn block_size(&self) -> gpt_disk_types::BlockSize {
            gpt_disk_types::BlockSize::new(512).unwrap()
        }

        fn num_blocks(&mut self) -> Result<u64, Self::Error> {
            Ok(8)
        }

        fn read_blocks(&mut self, lba: Lba, buffer: &mut [u8]) -> Result<(), Self::Error> {
            let offset = (lba.0 as usize) * 512;
            if offset + buffer.len() <= self.data.len() {
                buffer.copy_from_slice(&self.data[offset..offset + buffer.len()]);
            }
            Ok(())
        }

        fn write_blocks(&mut self, _lba: Lba, _buffer: &[u8]) -> Result<(), Self::Error> {
            Ok(())
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn test_lba_translation() {
        let mut ctx = IsoReadContext {
            chunk_lbas: [(0, 0); MAX_CHUNKS],
            chunk_sizes: [0; MAX_CHUNKS],
            num_chunks: 1,
            total_size: 1_000_000,
        };
        ctx.chunk_lbas[0] = (1000, 10000);
        ctx.chunk_sizes[0] = 1_000_000;

        let mut mock = MockBlockIo { data: [0; 4096] };
        let adapter = IsoBlockIoAdapter::new(ctx, &mut mock);

        // LBA 0 should translate to partition start + data offset
        let phys = adapter.translate_lba(0).unwrap();
        assert_eq!(phys, 1000 + 8192); // part_start + DATA_START_SECTOR

        // LBA 10 should be 10 sectors further
        let phys = adapter.translate_lba(10).unwrap();
        assert_eq!(phys, 1000 + 8192 + 10);
    }
}
