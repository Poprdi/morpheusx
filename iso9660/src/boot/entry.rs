//! El Torito boot catalog entry layout.

use crate::types::BootMediaType;

/// 32-byte catalog entry covering Initial/Default and Section formats.
#[repr(C, packed)]
pub struct BootEntry {
    /// 0x88 = bootable, 0x00 = not.
    pub boot_indicator: u8,
    /// Media type code; see `media_type()`.
    pub boot_media_type: u8,
    /// Real-mode load segment (0 = default 0x07C0).
    pub load_segment: u16,
    /// MBR partition type byte.
    pub system_type: u8,
    /// Unused; spec mandates zero.
    pub unused1: u8,
    /// Virtual sector count (512-byte units).
    pub sector_count: u16,
    /// Image start LBA on the disc (2048-byte sectors).
    pub load_rba: u32,
    /// Reserved tail.
    pub unused2: [u8; 20],
}

impl BootEntry {
    /// Boot indicator value marking the entry as bootable.
    pub const BOOTABLE: u8 = 0x88;

    /// Boot indicator value marking the entry as non-bootable.
    pub const NOT_BOOTABLE: u8 = 0x00;

    /// Whether the entry is flagged bootable.
    pub fn is_bootable(&self) -> bool {
        self.boot_indicator == Self::BOOTABLE
    }

    /// Decoded media type; unknown values fall back to `NoEmulation`.
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

    /// Image size in bytes: `sector_count * 512`.
    pub fn image_size(&self) -> u32 {
        self.sector_count as u32 * 512
    }
}
