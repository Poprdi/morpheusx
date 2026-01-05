//! Directory iteration
//!
//! Iterator for reading directory entries sequentially.

use crate::error::Result;
use crate::types::FileEntry;
use gpt_disk_io::BlockIo;

/// Directory iterator
pub struct DirectoryIterator<'a, B: BlockIo> {
    block_io: &'a mut B,
    extent_lba: u32,
    extent_len: u32,
    offset: usize,
}

impl<'a, B: BlockIo> DirectoryIterator<'a, B> {
    /// Create new directory iterator
    pub fn new(block_io: &'a mut B, extent_lba: u32, extent_len: u32) -> Self {
        Self {
            block_io,
            extent_lba,
            extent_len,
            offset: 0,
        }
    }
}

impl<'a, B: BlockIo> Iterator for DirectoryIterator<'a, B> {
    type Item = Result<FileEntry>;
    
    fn next(&mut self) -> Option<Self::Item> {
        // TODO: Implementation
        // 1. Read current sector if needed
        // 2. Parse directory record
        // 3. Advance offset
        // 4. Skip . and .. entries
        None
    }
}
