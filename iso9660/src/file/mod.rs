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
    let bytes_to_read = core::cmp::min(buffer.len(), file_size);
    
    if bytes_to_read == 0 {
        return Ok(0);
    }
    
    // Calculate number of sectors needed for the requested bytes
    let sector_count = bytes_to_read.div_ceil(SECTOR_SIZE);
    let start_lba = file.extent_lba as u64;
    
    // Allocate sector buffer once outside the loop
    let mut sector = [0u8; SECTOR_SIZE];
    
    // Read sectors
    for i in 0..sector_count {
        block_io.read_blocks(Lba(start_lba + i as u64), &mut sector)
            .map_err(|_| Iso9660Error::IoError)?;
        
        let offset = i * SECTOR_SIZE;
        // Copy what we need from this sector
        // The remaining bytes in the buffer might be less than a sector
        let remaining = bytes_to_read - offset;
        let len = core::cmp::min(SECTOR_SIZE, remaining);
        
        buffer[offset..offset + len].copy_from_slice(&sector[..len]);
    }
    
    Ok(bytes_to_read)
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
