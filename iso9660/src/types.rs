//! Common types and constants for ISO9660

// Vec is imported but not used in current stubs
#[allow(unused_imports)]
use alloc::vec::Vec;

/// ISO9660 sector size (always 2048 bytes)
pub const SECTOR_SIZE: usize = 2048;

/// Volume descriptor set starts at sector 16
pub const VOLUME_DESCRIPTOR_START: u64 = 16;

/// Maximum path length
pub const MAX_PATH_LENGTH: usize = 255;

/// Maximum directory depth
pub const MAX_DIRECTORY_DEPTH: usize = 8;

/// Volume descriptor type codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VolumeDescriptorType {
    /// Boot Record (El Torito)
    BootRecord = 0,
    /// Primary Volume Descriptor
    Primary = 1,
    /// Supplementary Volume Descriptor (Joliet)
    Supplementary = 2,
    /// Volume Partition Descriptor
    Partition = 3,
    /// Volume Descriptor Set Terminator
    Terminator = 255,
}

/// Parsed volume information
#[derive(Debug, Clone)]
pub struct VolumeInfo {
    /// Volume identifier (32 chars)
    pub volume_id: [u8; 32],

    /// Root directory extent location (LBA)
    pub root_extent_lba: u32,

    /// Root directory extent length (bytes)
    pub root_extent_len: u32,

    /// Logical block size (usually 2048)
    pub logical_block_size: u16,

    /// Volume space size (total sectors)
    pub volume_space_size: u32,

    /// El Torito boot catalog LBA (if present)
    pub boot_catalog_lba: Option<u32>,

    /// Whether Joliet extensions are present
    pub has_joliet: bool,

    /// Whether Rock Ridge extensions are present
    pub has_rock_ridge: bool,
}

/// File entry metadata
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// File identifier (name as UTF-8)
    pub name: alloc::string::String,

    /// File size in bytes
    pub size: u64,

    /// Extent location (LBA)
    pub extent_lba: u32,

    /// Data length (bytes)
    pub data_length: u32,

    /// File flags
    pub flags: FileFlags,

    /// File unit size (interleaved files)
    pub file_unit_size: u8,

    /// Interleave gap size
    pub interleave_gap: u8,
}

/// File flags from directory record
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFlags {
    /// Hidden file
    pub hidden: bool,

    /// Directory (not a file)
    pub directory: bool,

    /// Associated file
    pub associated: bool,

    /// Extended attribute record format
    pub extended_format: bool,

    /// Owner/group permissions in extended attributes
    pub extended_permissions: bool,

    /// Not final directory record for this file
    pub not_final: bool,
}

/// Boot image information (El Torito)
#[derive(Debug, Clone)]
pub struct BootImage {
    /// Bootable flag
    pub bootable: bool,

    /// Boot media type
    pub media_type: BootMediaType,

    /// Load segment (x86)
    pub load_segment: u16,

    /// System type
    pub system_type: u8,

    /// Sector count
    pub sector_count: u16,

    /// Virtual disk LBA
    pub load_rba: u32,

    /// Platform ID
    pub platform: BootPlatform,
}

/// Boot media type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BootMediaType {
    /// No emulation
    NoEmulation = 0,
    /// 1.2MB floppy
    Floppy12M = 1,
    /// 1.44MB floppy
    Floppy144M = 2,
    /// 2.88MB floppy
    Floppy288M = 3,
    /// Hard disk
    HardDisk = 4,
}

/// Boot platform ID
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BootPlatform {
    /// x86 PC
    X86 = 0,
    /// PowerPC
    PowerPC = 1,
    /// Mac
    Mac = 2,
    /// EFI
    Efi = 0xEF,
}
