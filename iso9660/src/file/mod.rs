//! File reading and extent management

pub mod reader;
pub mod metadata;
pub mod extent;

use crate::error::{Iso9660Error, Result};
use crate::types::{FileEntry, SECTOR_SIZE};
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Read file contents
///
/// # Arguments
/// * `block_io` - Block device
/// * `file` - File entry to read
/// * `buffer` - Buffer to read into
///
/// # Returns
/// Number of bytes read
pub fn read_file<B: BlockIo>(
    block_io: &mut B,
    file: &FileEntry,
    buffer: &mut [u8],
) -> Result<usize> {
    let file_size = file.size as usize;
    
    // Check buffer size
    if buffer.len() < file_size {
        return Err(Iso9660Error::ReadFailed);
    }
    
    // Calculate number of sectors to read
    let sector_count = file_size.div_ceil(SECTOR_SIZE);
    let start_lba = file.extent_lba as u64;
    
    // Read sectors
    for i in 0..sector_count {
        let mut sector = [0u8; SECTOR_SIZE];
        block_io.read_blocks(Lba(start_lba + i as u64), &mut sector)
            .map_err(|_| Iso9660Error::IoError)?;
        
        let offset = i * SECTOR_SIZE;
        let len = core::cmp::min(SECTOR_SIZE, file_size - offset);
        buffer[offset..offset + len].copy_from_slice(&sector[..len]);
    }
    
    Ok(file_size)
}

/// Read file into new Vec
///
/// Always available since we use `extern crate alloc`
pub fn read_file_vec<B: BlockIo>(
    block_io: &mut B,
    file: &FileEntry,
) -> Result<alloc::vec::Vec<u8>> {
    let mut buffer = alloc::vec![0u8; file.size as usize];
    read_file(block_io, file, &mut buffer)?;
    Ok(buffer)
}
