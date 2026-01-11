//! Boot catalog entry types
//!
//! Initial/Default, Section Header, and Section entries.

use crate::types::BootMediaType;

/// Boot Catalog Entry (32 bytes)
#[repr(C, packed)]
pub struct BootEntry {
    /// Boot indicator (0x88 = bootable, 0x00 = not bootable)
    pub boot_indicator: u8,

    /// Boot media type
    pub boot_media_type: u8,

    /// Load segment (0 = default 0x7C0)
    pub load_segment: u16,

    /// System type (partition type from MBR)
    pub system_type: u8,

    /// Unused
    pub unused1: u8,

    /// Sector count (virtual sectors, 512 bytes each)
    pub sector_count: u16,

    /// Load RBA (ISO sector, 2048 bytes)
    pub load_rba: u32,

    /// Unused (20 bytes)
    pub unused2: [u8; 20],
}

impl BootEntry {
    /// Bootable indicator
    pub const BOOTABLE: u8 = 0x88;

    /// Not bootable indicator
    pub const NOT_BOOTABLE: u8 = 0x00;

    /// Is this entry bootable?
    pub fn is_bootable(&self) -> bool {
        self.boot_indicator == Self::BOOTABLE
    }

    /// Parse boot media type
    pub fn media_type(&self) -> BootMediaType {
        match self.boot_media_type {
            0 => BootMediaType::NoEmulation,
            1 => BootMediaType::Floppy12M,
            2 => BootMediaType::Floppy144M,
            3 => BootMediaType::Floppy288M,
            4 => BootMediaType::HardDisk,
            _ => BootMediaType::NoEmulation,
        }
    }

    /// Get image size in bytes (sector_count * 512)
    pub fn image_size(&self) -> u32 {
        self.sector_count as u32 * 512
    }
}
