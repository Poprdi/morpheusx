//! Directory record header (ISO 9660 §9.1). Variable-length tail follows.

use crate::error::{Iso9660Error, Result};
use crate::types::FileFlags;

/// Fixed 33-byte header of a directory record. The file identifier,
/// optional pad byte, and system use area follow.
#[repr(C, packed)]
pub struct DirectoryRecord {
    /// Total record length in bytes.
    pub length: u8,
    /// Length of any extended attribute record.
    pub extended_attr_length: u8,
    /// Both-endian extent LBA (LE first 4 bytes, BE second 4).
    pub extent_lba: [u8; 8],
    /// Both-endian data length.
    pub data_length: [u8; 8],
    /// 7-byte recording timestamp (year-1900, mon, day, h, m, s, gmt-offset).
    pub recording_datetime: [u8; 7],
    /// File flag bits; see `FileFlags`.
    pub file_flags: u8,
    /// Interleave file unit size (0 = not interleaved).
    pub file_unit_size: u8,
    /// Interleave gap size.
    pub interleave_gap: u8,
    /// Both-endian volume sequence number.
    pub volume_sequence: [u8; 4],
    /// Length of the file identifier that follows the header.
    pub file_id_len: u8,
}

impl DirectoryRecord {
    /// Size of the fixed header before the file identifier.
    pub const MIN_LENGTH: u8 = 33;

    /// Reinterpret a byte slice as a `DirectoryRecord` after structural checks.
    pub fn parse(data: &[u8]) -> Result<&Self> {
        if data.len() < Self::MIN_LENGTH as usize {
            return Err(Iso9660Error::InvalidDirectoryRecord);
        }

        // SAFETY: length checked; the struct is repr(C, packed) with no padding
        // that requires alignment beyond u8.
        let record = unsafe { &*(data.as_ptr() as *const DirectoryRecord) };

        if record.length == 0 || record.length as usize > data.len() {
            return Err(Iso9660Error::InvalidDirectoryRecord);
        }

        if record.file_id_len as usize + Self::MIN_LENGTH as usize > record.length as usize {
            return Err(Iso9660Error::InvalidDirectoryRecord);
        }

        Ok(record)
    }

    /// Decoded extent LBA (uses the LE half of the both-endian field).
    pub fn get_extent_lba(&self) -> u32 {
        u32::from_le_bytes([
            self.extent_lba[0],
            self.extent_lba[1],
            self.extent_lba[2],
            self.extent_lba[3],
        ])
    }

    pub fn get_data_length(&self) -> u32 {
        u32::from_le_bytes([
            self.data_length[0],
            self.data_length[1],
            self.data_length[2],
            self.data_length[3],
        ])
    }

    pub fn get_flags(&self) -> FileFlags {
        FileFlags {
            hidden: self.file_flags & 0x01 != 0,
            directory: self.file_flags & 0x02 != 0,
            associated: self.file_flags & 0x04 != 0,
            extended_format: self.file_flags & 0x08 != 0,
            extended_permissions: self.file_flags & 0x10 != 0,
            not_final: self.file_flags & 0x80 != 0,
        }
    }

    pub fn is_directory(&self) -> bool {
        self.file_flags & 0x02 != 0
    }

    /// File identifier bytes immediately after the fixed header.
    pub fn file_identifier(&self) -> &[u8] {
        let start = 33;
        let len = self.file_id_len as usize;
        // SAFETY: parse() bounds-checked file_id_len against record.length.
        unsafe {
            let base_ptr = self as *const _ as *const u8;
            core::slice::from_raw_parts(base_ptr.add(start), len)
        }
    }
}
