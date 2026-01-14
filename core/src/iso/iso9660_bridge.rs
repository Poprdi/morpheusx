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

extern crate alloc;
use super::reader::IsoReadContext;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// ISO9660 sector size (what iso9660 crate expects)
const ISO_SECTOR_SIZE: usize = 2048;

/// Physical disk block size
const DISK_BLOCK_SIZE: usize = 512;

/// Ratio of ISO sectors to disk blocks
const BLOCKS_PER_ISO_SECTOR: usize = ISO_SECTOR_SIZE / DISK_BLOCK_SIZE;

/// Block I/O adapter that implements gpt_disk_io::BlockIo
///
/// This allows iso9660 to read from chunked ISO storage transparently.
/// The adapter translates virtual sector addresses to physical disk locations.
///
/// iso9660 uses 2048-byte sectors, but physical disk uses 512-byte blocks.
/// This adapter handles the translation.
pub struct IsoBlockIoAdapter<'a, B: BlockIo> {
    /// ISO read context (chunk partition info)
    ctx: IsoReadContext,
    /// Underlying block device
    block_io: &'a mut B,
}

impl<'a, B: BlockIo> IsoBlockIoAdapter<'a, B> {
    /// Create a new adapter
    ///
    /// # Arguments
    /// * `ctx` - ISO read context from IsoStorageManager
    /// * `block_io` - Underlying disk block device
    pub fn new(ctx: IsoReadContext, block_io: &'a mut B) -> Self {
        Self { ctx, block_io }
    }

    /// Get total size in bytes
    pub fn total_size(&self) -> u64 {
        self.ctx.total_size
    }

    /// Get total number of ISO sectors (2048-byte sectors)
    pub fn total_iso_sectors(&self) -> u64 {
        self.ctx.total_size / ISO_SECTOR_SIZE as u64
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

    /// Translate ISO byte offset to physical disk LBA (512-byte sectors)
    fn translate_byte_offset_to_disk_lba(&self, byte_offset: u64) -> Option<u64> {
        let (chunk_idx, offset_in_chunk) = self.find_chunk_for_offset(byte_offset)?;

        // Get partition start LBA from manifest
        let (part_start, _) = self.ctx.chunk_lbas[chunk_idx];

        // Calculate sector within the chunk
        // ISO data is written directly at partition start (no offset)
        let disk_sector_in_chunk = offset_in_chunk / DISK_BLOCK_SIZE as u64;

        // Physical LBA = partition start + sector offset
        let physical_lba = part_start + disk_sector_in_chunk;

        Some(physical_lba)
    }
}

impl<'a, B: BlockIo> BlockIo for IsoBlockIoAdapter<'a, B> {
    type Error = B::Error;

    fn block_size(&self) -> gpt_disk_types::BlockSize {
        // Report 2048-byte block size (ISO9660 sector size)
        gpt_disk_types::BlockSize::new(ISO_SECTOR_SIZE as u32).unwrap()
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.total_iso_sectors())
    }

    fn read_blocks(&mut self, start_lba: Lba, buffer: &mut [u8]) -> Result<(), Self::Error> {
        // iso9660 requests reads in 2048-byte sectors
        let num_iso_sectors = buffer.len() / ISO_SECTOR_SIZE;

        if num_iso_sectors == 0 {
            return Ok(());
        }

        // Try to batch contiguous reads for better performance
        // This is critical for large files like initrd (70MB+)
        let mut current_pos = 0usize;

        while current_pos < num_iso_sectors {
            let iso_sector = start_lba.0 + current_pos as u64;
            let byte_offset = iso_sector * ISO_SECTOR_SIZE as u64;

            // Check bounds
            if byte_offset >= self.ctx.total_size {
                // Read beyond EOF - fill remaining with zeros
                let buf_start = current_pos * ISO_SECTOR_SIZE;
                buffer[buf_start..].fill(0);
                break;
            }

            // Get the physical LBA for the start of this ISO sector
            let first_physical_lba = match self.translate_byte_offset_to_disk_lba(byte_offset) {
                Some(lba) => lba,
                None => {
                    // Chunk not found - fill this sector with zeros and continue
                    let buf_offset = current_pos * ISO_SECTOR_SIZE;
                    buffer[buf_offset..buf_offset + ISO_SECTOR_SIZE].fill(0);
                    current_pos += 1;
                    continue;
                }
            };

            // Determine how many contiguous ISO sectors we can read at once
            // Sectors are contiguous if they map to consecutive physical LBAs
            let mut batch_count = 1usize;

            while current_pos + batch_count < num_iso_sectors {
                let next_iso_sector = start_lba.0 + (current_pos + batch_count) as u64;
                let next_byte_offset = next_iso_sector * ISO_SECTOR_SIZE as u64;

                // Check bounds
                if next_byte_offset >= self.ctx.total_size {
                    break;
                }

                // Check if this sector is contiguous with the batch
                let expected_lba =
                    first_physical_lba + (batch_count * BLOCKS_PER_ISO_SECTOR) as u64;
                let actual_lba = match self.translate_byte_offset_to_disk_lba(next_byte_offset) {
                    Some(lba) => lba,
                    None => break, // End batch at chunk boundary
                };

                if actual_lba != expected_lba {
                    // Not contiguous (chunk boundary), end this batch
                    break;
                }

                batch_count += 1;
            }

            // Read the entire batch in one disk operation
            let buf_start = current_pos * ISO_SECTOR_SIZE;
            let buf_end = buf_start + batch_count * ISO_SECTOR_SIZE;

            self.block_io
                .read_blocks(Lba(first_physical_lba), &mut buffer[buf_start..buf_end])?;

            current_pos += batch_count;
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
    fn test_byte_offset_translation() {
        use crate::iso::MAX_CHUNKS;
        let mut ctx = IsoReadContext {
            chunk_lbas: [(0, 0); MAX_CHUNKS],
            chunk_sizes: [0; MAX_CHUNKS],
            num_chunks: 1,
            total_size: 1_000_000,
        };
        // Chunk starts at disk LBA 1000
        ctx.chunk_lbas[0] = (1000, 10000);
        ctx.chunk_sizes[0] = 1_000_000;

        let mut mock = MockBlockIo { data: [0; 4096] };
        let adapter = IsoBlockIoAdapter::new(ctx, &mut mock);

        // Byte offset 0 should translate to partition start
        let phys = adapter.translate_byte_offset_to_disk_lba(0).unwrap();
        assert_eq!(phys, 1000); // part_start

        // Byte offset 512 should be 1 disk sector further
        let phys = adapter.translate_byte_offset_to_disk_lba(512).unwrap();
        assert_eq!(phys, 1000 + 1);

        // Byte offset 2048 (1 ISO sector) should be 4 disk sectors from start
        let phys = adapter.translate_byte_offset_to_disk_lba(2048).unwrap();
        assert_eq!(phys, 1000 + 4);
    }
}
