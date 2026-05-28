//! `gpt_disk_io::BlockIo` bridge so iso9660 can read chunked ISO storage,
//! translating its 2048-byte sectors to 512-byte disk blocks.

extern crate alloc;
use super::reader::IsoReadContext;
use crate::fs::SECTOR_SIZE as DISK_BLOCK_SIZE;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

const ISO_SECTOR_SIZE: usize = 2048;
const BLOCKS_PER_ISO_SECTOR: usize = ISO_SECTOR_SIZE / DISK_BLOCK_SIZE;

pub struct IsoBlockIoAdapter<'a, B: BlockIo> {
    ctx: IsoReadContext,
    block_io: &'a mut B,
}

impl<'a, B: BlockIo> IsoBlockIoAdapter<'a, B> {
    pub fn new(ctx: IsoReadContext, block_io: &'a mut B) -> Self {
        Self { ctx, block_io }
    }

    pub fn total_size(&self) -> u64 {
        self.ctx.total_size
    }

    pub fn total_iso_sectors(&self) -> u64 {
        self.ctx.total_size / ISO_SECTOR_SIZE as u64
    }

    /// Returns (chunk index, offset within chunk) for a byte offset.
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

    /// ISO byte offset to physical disk LBA (512-byte sectors).
    fn translate_byte_offset_to_disk_lba(&self, byte_offset: u64) -> Option<u64> {
        let (chunk_idx, offset_in_chunk) = self.find_chunk_for_offset(byte_offset)?;

        // ISO data written directly at partition start, no offset.
        let (part_start, _) = self.ctx.chunk_lbas[chunk_idx];
        let disk_sector_in_chunk = offset_in_chunk / DISK_BLOCK_SIZE as u64;

        Some(part_start + disk_sector_in_chunk)
    }
}

impl<'a, B: BlockIo> BlockIo for IsoBlockIoAdapter<'a, B> {
    type Error = B::Error;

    fn block_size(&self) -> gpt_disk_types::BlockSize {
        gpt_disk_types::BlockSize::new(ISO_SECTOR_SIZE as u32).unwrap()
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.total_iso_sectors())
    }

    fn read_blocks(&mut self, start_lba: Lba, buffer: &mut [u8]) -> Result<(), Self::Error> {
        let num_iso_sectors = buffer.len() / ISO_SECTOR_SIZE;

        if num_iso_sectors == 0 {
            return Ok(());
        }

        // Batch contiguous reads; matters for large files (initrd 70 MB+).
        let mut current_pos = 0usize;

        while current_pos < num_iso_sectors {
            let iso_sector = start_lba.0 + current_pos as u64;
            let byte_offset = iso_sector * ISO_SECTOR_SIZE as u64;

            if byte_offset >= self.ctx.total_size {
                // Past EOF: zero-fill remainder.
                let buf_start = current_pos * ISO_SECTOR_SIZE;
                buffer[buf_start..].fill(0);
                break;
            }

            let first_physical_lba = match self.translate_byte_offset_to_disk_lba(byte_offset) {
                Some(lba) => lba,
                None => {
                    // Chunk gap: zero this sector and continue.
                    let buf_offset = current_pos * ISO_SECTOR_SIZE;
                    buffer[buf_offset..buf_offset + ISO_SECTOR_SIZE].fill(0);
                    current_pos += 1;
                    continue;
                },
            };

            // Extend batch while sectors map to consecutive physical LBAs.
            let mut batch_count = 1usize;

            while current_pos + batch_count < num_iso_sectors {
                let next_iso_sector = start_lba.0 + (current_pos + batch_count) as u64;
                let next_byte_offset = next_iso_sector * ISO_SECTOR_SIZE as u64;

                if next_byte_offset >= self.ctx.total_size {
                    break;
                }

                let expected_lba =
                    first_physical_lba + (batch_count * BLOCKS_PER_ISO_SECTOR) as u64;
                let actual_lba = match self.translate_byte_offset_to_disk_lba(next_byte_offset) {
                    Some(lba) => lba,
                    None => break,
                };

                if actual_lba != expected_lba {
                    break;
                }

                batch_count += 1;
            }

            let buf_start = current_pos * ISO_SECTOR_SIZE;
            let buf_end = buf_start + batch_count * ISO_SECTOR_SIZE;

            self.block_io
                .read_blocks(Lba(first_physical_lba), &mut buffer[buf_start..buf_end])?;

            current_pos += batch_count;
        }

        Ok(())
    }

    fn write_blocks(&mut self, _start_lba: Lba, _buffer: &[u8]) -> Result<(), Self::Error> {
        // ISO storage is read-only; iso9660 never writes.
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.block_io.flush()
    }
}

/// Couples ISO storage access with iso9660 parsing.
pub struct ChunkedIso<'a, B: BlockIo> {
    adapter: IsoBlockIoAdapter<'a, B>,
}

impl<'a, B: BlockIo> ChunkedIso<'a, B> {
    pub fn new(ctx: IsoReadContext, block_io: &'a mut B) -> Self {
        Self {
            adapter: IsoBlockIoAdapter::new(ctx, block_io),
        }
    }

    pub fn block_io(&mut self) -> &mut IsoBlockIoAdapter<'a, B> {
        &mut self.adapter
    }

    pub fn total_size(&self) -> u64 {
        self.adapter.total_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt;

    #[derive(Debug)]
    struct MockError;

    impl fmt::Display for MockError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "MockError")
        }
    }

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
        ctx.chunk_lbas[0] = (1000, 10000);
        ctx.chunk_sizes[0] = 1_000_000;

        let mut mock = MockBlockIo { data: [0; 4096] };
        let adapter = IsoBlockIoAdapter::new(ctx, &mut mock);

        let phys = adapter.translate_byte_offset_to_disk_lba(0).unwrap();
        assert_eq!(phys, 1000);

        let phys = adapter.translate_byte_offset_to_disk_lba(512).unwrap();
        assert_eq!(phys, 1000 + 1);

        // 1 ISO sector (2048) = 4 disk sectors.
        let phys = adapter.translate_byte_offset_to_disk_lba(2048).unwrap();
        assert_eq!(phys, 1000 + 4);
    }
}
