//! El Torito boot catalog validation entry.

use crate::utils::checksum;

/// 32-byte validation entry preceding the first boot entry.
#[repr(C, packed)]
pub struct ValidationEntry {
    /// Header ID; must be 0x01.
    pub header_id: u8,
    /// Platform ID byte.
    pub platform_id: u8,
    /// Reserved; zero.
    pub reserved: u16,
    /// Manufacturer/developer ID, space-padded.
    pub id_string: [u8; 24],
    /// 16-bit one's-complement checksum word.
    pub checksum: u16,
    /// Magic key {0x55, 0xAA}.
    pub key: [u8; 2],
}

impl ValidationEntry {
    pub const HEADER_ID: u8 = 0x01;

    pub const KEY_BYTES: [u8; 2] = [0x55, 0xAA];

    /// Header, key, and checksum all valid.
    pub fn is_valid(&self) -> bool {
        self.header_id == Self::HEADER_ID && self.key == Self::KEY_BYTES && self.verify_checksum()
    }

    fn verify_checksum(&self) -> bool {
        // SAFETY: repr(C, packed) struct of exactly 32 bytes.
        let bytes = unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, 32) };
        checksum::verify_checksum_16(bytes)
    }
}
