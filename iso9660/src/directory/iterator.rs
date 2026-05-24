//! Sequential iterator over directory records.

use crate::directory::record::DirectoryRecord;
use crate::error::{Iso9660Error, Result};
use crate::types::{FileEntry, SECTOR_SIZE};
use crate::utils::string;
use alloc::boxed::Box;
use alloc::string::String;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Walks directory records in an extent, one sector at a time.
pub struct DirectoryIterator<'a, B: BlockIo> {
    block_io: &'a mut B,
    extent_lba: u32,
    extent_len: u32,
    offset: usize,
    current_sector: Box<[u8; SECTOR_SIZE]>,
    current_sector_lba: Option<u64>,
}

impl<'a, B: BlockIo> DirectoryIterator<'a, B> {
    /// Construct an iterator over the directory extent at `extent_lba`.
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
            if self.offset >= self.extent_len as usize {
                return None;
            }

            let sector_offset = self.offset / SECTOR_SIZE;
            let lba = self.extent_lba as u64 + sector_offset as u64;
            let offset_in_sector = self.offset % SECTOR_SIZE;

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

            let sector_data = &self.current_sector[offset_in_sector..];

            // Records do not straddle sectors; a zero length byte means the
            // remainder is padding (ISO 9660 §6.8.1.1).
            if sector_data.is_empty() || sector_data[0] == 0 {
                self.offset = (sector_offset + 1) * SECTOR_SIZE;
                continue;
            }

            let record = match DirectoryRecord::parse(sector_data) {
                Ok(r) => r,
                Err(e) => return Some(Err(e)),
            };

            self.offset += record.length as usize;

            let file_id = record.file_identifier();

            // 0x00 = ".", 0x01 = ".." (ISO 9660 §7.6.2)
            let name = if file_id.len() == 1 && file_id[0] == 0 {
                String::from(".")
            } else if file_id.len() == 1 && file_id[0] == 1 {
                String::from("..")
            } else {
                match string::dchars_to_str(file_id) {
                    Ok(s) => String::from(string::strip_version(s)),
                    Err(_) => {
                        let s = String::from_utf8_lossy(file_id);
                        String::from(string::strip_version(&s))
                    }
                }
            };

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
