//! ISO 9660 types and constants.

#[allow(unused_imports)]
use alloc::vec::Vec;

/// Logical sector size. Fixed by spec.
pub const SECTOR_SIZE: usize = 2048;

/// Volume descriptor set begins at sector 16 (ISO 9660 §6.2.1).
pub const VOLUME_DESCRIPTOR_START: u64 = 16;

pub const MAX_PATH_LENGTH: usize = 255;

pub const MAX_DIRECTORY_DEPTH: usize = 8;

/// Volume descriptor type code (ISO 9660 §8.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VolumeDescriptorType {
    /// Boot Record (El Torito)
    BootRecord = 0,
    /// Primary Volume Descriptor
    Primary = 1,
    /// Supplementary (Joliet)
    Supplementary = 2,
    /// Volume Partition Descriptor
    Partition = 3,
    /// Set Terminator
    Terminator = 255,
}

/// Parsed volume metadata.
#[derive(Debug, Clone)]
pub struct VolumeInfo {
    /// d-characters, space-padded
    pub volume_id: [u8; 32],
    /// Root directory extent LBA
    pub root_extent_lba: u32,
    /// Root directory extent length in bytes
    pub root_extent_len: u32,
    /// Logical block size; almost always 2048
    pub logical_block_size: u16,
    /// Total sectors in volume
    pub volume_space_size: u32,
    /// El Torito boot catalog LBA, if any
    pub boot_catalog_lba: Option<u32>,
    /// Joliet SVD present
    pub has_joliet: bool,
    /// Rock Ridge SUSP/RRIP detected
    pub has_rock_ridge: bool,
}

/// Directory entry metadata.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Decoded name (UTF-8)
    pub name: alloc::string::String,
    /// Size in bytes
    pub size: u64,
    /// Extent LBA
    pub extent_lba: u32,
    /// Data length in bytes
    pub data_length: u32,
    /// Directory record flag bits
    pub flags: FileFlags,
    /// Interleave file unit size (0 = not interleaved)
    pub file_unit_size: u8,
    /// Interleave gap size
    pub interleave_gap: u8,
}

/// Directory record flag bits (ISO 9660 §9.1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFlags {
    /// Hidden from normal listings.
    pub hidden: bool,
    /// Entry describes a directory.
    pub directory: bool,
    /// Associated file (xattr-like).
    pub associated: bool,
    /// Record format defined in EAR.
    pub extended_format: bool,
    /// Permissions defined in EAR.
    pub extended_permissions: bool,
    /// Multi-extent: more records follow for this file.
    pub not_final: bool,
}

/// El Torito boot image descriptor.
#[derive(Debug, Clone)]
pub struct BootImage {
    /// Bootable flag from the boot entry.
    pub bootable: bool,
    /// Boot media emulation type.
    pub media_type: BootMediaType,
    /// x86 real-mode load segment.
    pub load_segment: u16,
    /// System type byte copied from the partition table.
    pub system_type: u8,
    /// Number of 512-byte virtual sectors loaded.
    pub sector_count: u16,
    /// Image LBA on the disc.
    pub load_rba: u32,
    /// Platform ID from the section header.
    pub platform: BootPlatform,
}

/// El Torito boot media type (§2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BootMediaType {
    /// No emulation; raw load.
    NoEmulation = 0,
    /// 1.2 MB floppy emulation.
    Floppy12M = 1,
    /// 1.44 MB floppy emulation.
    Floppy144M = 2,
    /// 2.88 MB floppy emulation.
    Floppy288M = 3,
    /// Hard disk emulation.
    HardDisk = 4,
}

/// El Torito platform ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BootPlatform {
    /// 80x86.
    X86 = 0,
    /// PowerPC.
    PowerPC = 1,
    /// Mac.
    Mac = 2,
    /// UEFI (0xEF per spec).
    Efi = 0xEF,
}
