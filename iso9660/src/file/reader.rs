//! Sector-buffered seek/read interface for ISO 9660 files.

use crate::error::{Iso9660Error, Result};
use crate::types::{FileEntry, SECTOR_SIZE};
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Streaming reader for files too large to slurp into memory.
pub struct FileReader<'a, B: BlockIo> {
    block_io: &'a mut B,
    file: FileEntry,
    position: u64,
}

impl<'a, B: BlockIo> FileReader<'a, B> {
    /// New reader positioned at offset 0.
    pub fn new(block_io: &'a mut B, file: FileEntry) -> Self {
        Self {
            block_io,
            file,
            position: 0,
        }
    }

    /// Read from the current position. Returns 0 at EOF.
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        if self.position >= self.file.size {
            return Ok(0);
        }

        let remaining = (self.file.size - self.position) as usize;
        let to_read = buffer.len().min(remaining);

        if to_read == 0 {
            return Ok(0);
        }

        let start_sector = (self.position / SECTOR_SIZE as u64) as u32;
        let offset_in_sector = (self.position % SECTOR_SIZE as u64) as usize;

        let mut bytes_read = 0;
        let mut sector_buf = [0u8; SECTOR_SIZE];
        let mut current_sector = start_sector;
        let mut current_offset = offset_in_sector;

        while bytes_read < to_read {
            let lba = Lba(self.file.extent_lba as u64 + current_sector as u64);
            self.block_io
                .read_blocks(lba, &mut sector_buf)
                .map_err(|_| Iso9660Error::IoError)?;

            let available = SECTOR_SIZE - current_offset;
            let chunk_size = available.min(to_read - bytes_read);

            buffer[bytes_read..bytes_read + chunk_size]
                .copy_from_slice(&sector_buf[current_offset..current_offset + chunk_size]);

            bytes_read += chunk_size;
            current_sector += 1;
            current_offset = 0;
        }

        self.position += bytes_read as u64;
        Ok(bytes_read)
    }

    /// Seek to an absolute byte position, clamped to file size.
    pub fn seek(&mut self, pos: u64) {
        self.position = pos.min(self.file.size);
    }

    /// Seek relative to current position; saturates at 0 and file size.
    pub fn seek_relative(&mut self, offset: i64) {
        let new_pos = if offset < 0 {
            self.position.saturating_sub((-offset) as u64)
        } else {
            self.position.saturating_add(offset as u64)
        };
        self.position = new_pos.min(self.file.size);
    }

    /// Current read offset, in bytes.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Total file size, in bytes.
    pub fn size(&self) -> u64 {
        self.file.size
    }

    /// True once the read position has reached end of file.
    pub fn is_eof(&self) -> bool {
        self.position >= self.file.size
    }

    /// Bytes left until EOF.
    pub fn remaining(&self) -> u64 {
        self.file.size.saturating_sub(self.position)
    }
}
