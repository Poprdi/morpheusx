//! Directory iteration
//!
//! Iterator for reading directory entries sequentially.

use crate::directory::record::DirectoryRecord;
use crate::error::{Iso9660Error, Result};
use crate::types::{FileEntry, SECTOR_SIZE};
use crate::utils::string;
use alloc::boxed::Box;
use alloc::string::String;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Directory iterator
pub struct DirectoryIterator<'a, B: BlockIo> {
    block_io: &'a mut B,
    extent_lba: u32,
    extent_len: u32,
    offset: usize,
    current_sector: Box<[u8; SECTOR_SIZE]>,
    current_sector_lba: Option<u64>,
}

impl<'a, B: BlockIo> DirectoryIterator<'a, B> {
    /// Create new directory iterator
    pub fn new(block_io: &'a mut B, extent_lba: u32, extent_len: u32) -> Self {
        Self {
            block_io,
            extent_lba,
            extent_len,
            offset: 0,
            current_sector: Box::new([0u8; SECTOR_SIZE]),
            current_sector_lba: None,
        }
    }
}

impl<'a, B: BlockIo> Iterator for DirectoryIterator<'a, B> {
    type Item = Result<FileEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Check if we've read all directory data
            if self.offset >= self.extent_len as usize {
                return None;
            }

            // Calculate current LBA and offset within sector
            let sector_offset = self.offset / SECTOR_SIZE;
            let lba = self.extent_lba as u64 + sector_offset as u64;
            let offset_in_sector = self.offset % SECTOR_SIZE;

            // Read sector if needed
            if self.current_sector_lba != Some(lba) {
                if self
                    .block_io
                    .read_blocks(Lba(lba), self.current_sector.as_mut())
                    .is_err()
                {
                    return Some(Err(Iso9660Error::IoError));
                }
                self.current_sector_lba = Some(lba);
            }

            // Get remaining data in current sector
            let sector_data = &self.current_sector[offset_in_sector..];

            // Check for zero-length record (skip to next sector)
            if sector_data.is_empty() || sector_data[0] == 0 {
                // Move to next sector
                let next_sector_offset = (sector_offset + 1) * SECTOR_SIZE;
                self.offset = next_sector_offset;
                continue;
            }

            // Parse directory record
            let record = match DirectoryRecord::parse(sector_data) {
                Ok(r) => r,
                Err(e) => return Some(Err(e)),
            };

            // Advance offset by record length
            self.offset += record.length as usize;

            // Convert file identifier to string
            let file_id = record.file_identifier();

            // Handle special directory entries
            let name = if file_id.len() == 1 && file_id[0] == 0 {
                String::from(".")
            } else if file_id.len() == 1 && file_id[0] == 1 {
                String::from("..")
            } else {
                match string::dchars_to_str(file_id) {
                    Ok(s) => {
                        // Strip version suffix (e.g., ";1")
                        let stripped = string::strip_version(s);
                        String::from(stripped)
                    }
                    Err(_) => {
                        // If not valid UTF-8, use lossy conversion
                        let s = String::from_utf8_lossy(file_id);
                        let stripped = string::strip_version(&s);
                        String::from(stripped)
                    }
                }
            };

            // Build FileEntry
            let entry = FileEntry {
                name,
                size: record.get_data_length() as u64,
                extent_lba: record.get_extent_lba(),
                data_length: record.get_data_length(),
                flags: record.get_flags(),
                file_unit_size: record.file_unit_size,
                interleave_gap: record.interleave_gap,
            };

            return Some(Ok(entry));
        }
    }
}
