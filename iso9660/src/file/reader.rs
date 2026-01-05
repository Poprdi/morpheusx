//! File reader implementation

use crate::error::Result;
use crate::types::FileEntry;
use gpt_disk_io::BlockIo;

/// Buffered file reader
#[allow(dead_code)]  // Stub implementation, fields will be used
pub struct FileReader<'a, B: BlockIo> {
    block_io: &'a mut B,
    file: FileEntry,
    position: u64,
}

impl<'a, B: BlockIo> FileReader<'a, B> {
    /// Create new file reader
    pub fn new(block_io: &'a mut B, file: FileEntry) -> Self {
        Self {
            block_io,
            file,
            position: 0,
        }
    }
    
    /// Read bytes from current position
    pub fn read(&mut self, _buffer: &mut [u8]) -> Result<usize> {
        // TODO: Implementation
        Ok(0)
    }
    
    /// Seek to position
    pub fn seek(&mut self, pos: u64) {
        self.position = pos;
    }
    
    /// Get current position
    pub fn position(&self) -> u64 {
        self.position
    }
    
    /// Get file size
    pub fn size(&self) -> u64 {
        self.file.size
    }
}
