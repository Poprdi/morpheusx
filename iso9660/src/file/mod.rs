//! File reading and extent management

pub mod reader;
pub mod metadata;
pub mod extent;

use crate::error::{Iso9660Error, Result};
use crate::types::FileEntry;
use gpt_disk_io::BlockIo;

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
    _block_io: &mut B,
    _file: &FileEntry,
    _buffer: &mut [u8],
) -> Result<usize> {
    // TODO: Implementation
    // 1. Read extents sequentially
    // 2. Handle fragmentation if any
    // 3. Respect file length
    
    Err(Iso9660Error::ReadFailed)
}

/// Read file into new Vec
///
/// Always available since we use `extern crate alloc`
pub fn read_file_vec<B: BlockIo>(
    _block_io: &mut B,
    _file: &FileEntry,
) -> Result<alloc::vec::Vec<u8>> {
    // TODO: Allocate and read
    Err(Iso9660Error::ReadFailed)
}
