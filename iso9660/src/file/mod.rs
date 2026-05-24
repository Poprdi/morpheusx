//! File reading helpers.

pub mod extent;
pub mod metadata;
pub mod reader;

use crate::error::{Iso9660Error, Result};
use crate::types::{FileEntry, SECTOR_SIZE};
use alloc::vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Cap each `read_blocks` at 64 KiB. UEFI BlockIo implementations frequently
/// reject larger transfers (max transfer size is often 64-128 KiB).
const MAX_SECTORS_PER_READ: usize = 32;

/// Read up to `buffer.len()` bytes of `file` into `buffer`. Returns bytes read.
pub fn read_file<B: BlockIo>(
    block_io: &mut B,
    file: &FileEntry,
    buffer: &mut [u8],
) -> Result<usize> {
    let file_size = file.size as usize;
    let bytes_to_read = core::cmp::min(buffer.len(), file_size);

    if bytes_to_read == 0 {
        return Ok(0);
    }

    let total_sectors = bytes_to_read.div_ceil(SECTOR_SIZE);
    let start_lba = file.extent_lba as u64;

    let mut sectors_read = 0usize;

    while sectors_read < total_sectors {
        let remaining_sectors = total_sectors - sectors_read;
        let chunk_sectors = core::cmp::min(remaining_sectors, MAX_SECTORS_PER_READ);
        let chunk_bytes = chunk_sectors * SECTOR_SIZE;

        let buf_offset = sectors_read * SECTOR_SIZE;
        let buf_end = core::cmp::min(buf_offset + chunk_bytes, bytes_to_read);
        let remaining_buf = buf_end - buf_offset;

        if remaining_buf >= chunk_bytes {
            block_io
                .read_blocks(
                    Lba(start_lba + sectors_read as u64),
                    &mut buffer[buf_offset..buf_offset + chunk_bytes],
                )
                .map_err(|_| Iso9660Error::IoError)?;
        } else {
            // Tail sector(s) overshoot the requested length; stage and copy.
            let mut temp = vec![0u8; chunk_bytes];
            block_io
                .read_blocks(Lba(start_lba + sectors_read as u64), &mut temp)
                .map_err(|_| Iso9660Error::IoError)?;
            buffer[buf_offset..buf_end].copy_from_slice(&temp[..remaining_buf]);
        }

        sectors_read += chunk_sectors;
    }

    Ok(bytes_to_read)
}

/// Allocate a `Vec` sized to the file and read it in full.
pub fn read_file_vec<B: BlockIo>(
    block_io: &mut B,
    file: &FileEntry,
) -> Result<alloc::vec::Vec<u8>> {
    let mut buffer = alloc::vec![0u8; file.size as usize];
    read_file(block_io, file, &mut buffer)?;
    Ok(buffer)
}
