//! FAT32 on-disk constants and engine-facing return types.

use alloc::string::String;

/// FAT32 EOC range: any link >= this ends a chain. Top 4 bits are reserved,
/// so compares mask to 28 bits.
pub const FAT32_EOC: u32 = 0x0FFF_FFF8;
pub const FAT32_BAD: u32 = 0x0FFF_FFF7;
pub const FAT_ENTRY_MASK: u32 = 0x0FFF_FFFF;

/// First two FAT slots are reserved; data clusters are numbered from 2.
pub const FIRST_DATA_CLUSTER: u32 = 2;

pub const DIR_ENTRY_SIZE: usize = 32;
pub const ENTRY_FREE: u8 = 0xE5;
pub const ENTRY_END: u8 = 0x00;

/// Directory-entry attribute byte bits.
pub const ATTR_READ_ONLY: u8 = 0x01;
pub const ATTR_HIDDEN: u8 = 0x02;
pub const ATTR_SYSTEM: u8 = 0x04;
pub const ATTR_VOLUME_ID: u8 = 0x08;
pub const ATTR_DIRECTORY: u8 = 0x10;
pub const ATTR_ARCHIVE: u8 = 0x20;
/// An LFN slot is flagged by all four of these set at once.
pub const ATTR_LONG_NAME: u8 = ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID;

/// Max UTF-16 code units an LFN can span (20 slots * 13 units).
pub const LFN_MAX_UNITS: usize = 260;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Directory,
}

/// One resolved directory entry (8.3 or reassembled LFN).
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: u64,
    pub start_cluster: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub file_type: FileType,
    pub size: u64,
    pub start_cluster: u32,
}
