//! File reading and extent management

pub mod extent;
pub mod metadata;
pub mod reader;

use crate::error::{Iso9660Error, Result};
use crate::types::{FileEntry, SECTOR_SIZE};
use alloc::vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Maximum sectors to read in a single I/O operation
/// 32 sectors = 64KB per read - safe for UEFI firmware transfer limits
/// Many UEFI implementations have max transfer sizes of 64-128KB
/// For 11MB kernel: ~172 reads, for 70MB initrd: ~1094 reads
const MAX_SECTORS_PER_READ: usize = 32;

// Progress logging disabled in standalone iso9660 crate (no logger dependency)
// const PROGRESS_INTERVAL_BYTES: usize = 4 * 1024 * 1024; // 4 MiB

/// Read file contents into a buffer
///
/// Reads file data from the block device into the provided buffer.
/// Returns the number of bytes actually read (may be less if file is smaller than buffer).
///
/// Uses bulk reads for improved performance with large files.
///
/// # Arguments
/// * `block_io` - Block device
/// * `file` - File entry to read from
/// * `buffer` - Buffer to read into
///
/// # Returns
/// Number of bytes read
///
/// # Example
/// ```ignore
/// use iso9660::{mount, find_file, read_file};
///
/// let volume = mount(&mut block_io, 0)?;
/// let file = find_file(&mut block_io, &volume, "/boot/vmlinuz")?;
/// let mut buffer = vec![0u8; file.size as usize];
/// read_file(&mut block_io, &file, &mut buffer)?;
/// ```
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

    // Calculate number of sectors needed for the requested bytes
    let total_sectors = bytes_to_read.div_ceil(SECTOR_SIZE);
    let start_lba = file.extent_lba as u64;

    // Read in chunks of MAX_SECTORS_PER_READ for efficiency
    let mut sectors_read = 0usize;
    // let mut bytes_reported = 0usize;

    while sectors_read < total_sectors {
        let remaining_sectors = total_sectors - sectors_read;
        let chunk_sectors = core::cmp::min(remaining_sectors, MAX_SECTORS_PER_READ);
        let chunk_bytes = chunk_sectors * SECTOR_SIZE;

        let buf_offset = sectors_read * SECTOR_SIZE;
        let buf_end = core::cmp::min(buf_offset + chunk_bytes, bytes_to_read);

        // For the last chunk, we might need to read into a temp buffer
        // if the remaining buffer space isn't a multiple of SECTOR_SIZE
        let remaining_buf = buf_end - buf_offset;

        if remaining_buf >= chunk_bytes {
            // Can read directly into buffer
            block_io
                .read_blocks(
                    Lba(start_lba + sectors_read as u64),
                    &mut buffer[buf_offset..buf_offset + chunk_bytes],
                )
                .map_err(|_| Iso9660Error::IoError)?;
        } else {
            // Last partial chunk - need temp buffer for full sector read
            let mut temp = vec![0u8; chunk_bytes];
            block_io
                .read_blocks(Lba(start_lba + sectors_read as u64), &mut temp)
                .map_err(|_| Iso9660Error::IoError)?;
            buffer[buf_offset..buf_end].copy_from_slice(&temp[..remaining_buf]);
        }

        sectors_read += chunk_sectors;

        // Progress logging disabled (no logger in iso9660 crate)
    }

    Ok(bytes_to_read)
}

/// Read file into new Vec (convenience function)
///
/// Allocates a Vec sized to the file and reads all contents.
/// Useful when you don't have a pre-allocated buffer.
///
/// # Arguments
/// * `block_io` - Block device
/// * `file` - File entry to read
///
/// # Returns
/// Vec containing the entire file contents
///
/// # Example
/// ```ignore
/// use iso9660::{mount, find_file, read_file_vec};
///
/// let volume = mount(&mut block_io, 0)?;
/// let file = find_file(&mut block_io, &volume, "/boot/vmlinuz")?;
/// let kernel = read_file_vec(&mut block_io, &file)?;
/// println!("Kernel: {} bytes", kernel.len());
/// ```
pub fn read_file_vec<B: BlockIo>(
    block_io: &mut B,
    file: &FileEntry,
) -> Result<alloc::vec::Vec<u8>> {
    let mut buffer = alloc::vec![0u8; file.size as usize];
    read_file(block_io, file, &mut buffer)?;
    Ok(buffer)
}
