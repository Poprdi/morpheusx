//! El Torito platform ID decoding.

use crate::types::BootPlatform;

impl BootPlatform {
    /// 80x86.
    pub const X86: u8 = 0x00;
    pub const POWER_PC: u8 = 0x01;
    /// Macintosh platform ID.
    pub const MAC: u8 = 0x02;
    /// UEFI.
    pub const EFI: u8 = 0xEF;

    /// Decode validation-entry platform ID; unknown values fall back to x86.
    pub fn from_id(id: u8) -> Self {
        match id {
            0x00 => BootPlatform::X86,
            0x01 => BootPlatform::PowerPC,
            0x02 => BootPlatform::Mac,
            0xEF => BootPlatform::Efi,
            _ => BootPlatform::X86,
        }
    }
}
