//! Directory Record structure
//!
//! Directory records describe files and subdirectories.

use crate::error::{Iso9660Error, Result};
use crate::types::FileFlags;

/// Directory Record (variable length)
#[repr(C, packed)]
pub struct DirectoryRecord {
    /// Length of directory record (BP 1)
    pub length: u8,

    /// Extended attribute record length (BP 2)
    pub extended_attr_length: u8,

    /// Extent location (both-endian 32-bit) (BP 3-10)
    pub extent_lba: [u8; 8],

    /// Data length (both-endian 32-bit) (BP 11-18)
    pub data_length: [u8; 8],

    /// Recording date and time (7 bytes) (BP 19-25)
    pub recording_datetime: [u8; 7],

    /// File flags (BP 26)
    pub file_flags: u8,

    /// File unit size (interleaved files) (BP 27)
    pub file_unit_size: u8,

    /// Interleave gap size (BP 28)
    pub interleave_gap: u8,

    /// Volume sequence number (both-endian 16-bit) (BP 29-32)
    pub volume_sequence: [u8; 4],

    /// File identifier length (BP 33)
    pub file_id_len: u8,
    // Followed by:
    // - File identifier (file_id_len bytes)
    // - Padding field (1 byte if file_id_len is even)
    // - System use area (variable)
}

impl DirectoryRecord {
    /// Minimum record length (33 bytes header)
    pub const MIN_LENGTH: u8 = 33;

    /// Parse directory record from bytes
    pub fn parse(data: &[u8]) -> Result<&Self> {
        // Validate minimum length
        if data.len() < Self::MIN_LENGTH as usize {
            return Err(Iso9660Error::InvalidDirectoryRecord);
        }

        // Cast to struct
        let record = unsafe { &*(data.as_ptr() as *const DirectoryRecord) };

        // Validate record length
        if record.length == 0 || record.length as usize > data.len() {
            return Err(Iso9660Error::InvalidDirectoryRecord);
        }

        // Validate file identifier length
        if record.file_id_len as usize + Self::MIN_LENGTH as usize > record.length as usize {
            return Err(Iso9660Error::InvalidDirectoryRecord);
        }

        Ok(record)
    }

    /// Get extent LBA (little-endian part of both-endian field)
    pub fn get_extent_lba(&self) -> u32 {
        u32::from_le_bytes([
            self.extent_lba[0],
            self.extent_lba[1],
            self.extent_lba[2],
            self.extent_lba[3],
        ])
    }

    /// Get data length (little-endian part)
    pub fn get_data_length(&self) -> u32 {
        u32::from_le_bytes([
            self.data_length[0],
            self.data_length[1],
            self.data_length[2],
            self.data_length[3],
        ])
    }

    /// Parse file flags
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

    /// Is this a directory?
    pub fn is_directory(&self) -> bool {
        self.file_flags & 0x02 != 0
    }

    /// Get file identifier bytes
    pub fn file_identifier(&self) -> &[u8] {
        // File identifier starts at offset 33 (after fixed header)
        let start = 33;
        let len = self.file_id_len as usize;

        // Safety: we validated file_id_len in parse()
        unsafe {
            let base_ptr = self as *const _ as *const u8;
            core::slice::from_raw_parts(base_ptr.add(start), len)
        }
    }
}
