//! Boot platform identifiers

use crate::types::BootPlatform;

impl BootPlatform {
    /// x86 (PC-compatible)
    pub const X86: u8 = 0x00;

    /// PowerPC
    pub const POWER_PC: u8 = 0x01;

    /// Mac
    pub const MAC: u8 = 0x02;

    /// EFI
    pub const EFI: u8 = 0xEF;

    /// Parse from validation entry platform ID
    pub fn from_id(id: u8) -> Self {
        match id {
            0x00 => BootPlatform::X86,
            0x01 => BootPlatform::PowerPC,
            0x02 => BootPlatform::Mac,
            0xEF => BootPlatform::Efi,
            _ => BootPlatform::X86, // Default to x86
        }
    }
}
