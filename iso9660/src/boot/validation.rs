//! Boot catalog validation entry
//!
//! The validation entry verifies catalog integrity via checksum.

use crate::utils::checksum;

/// Validation Entry (32 bytes)
#[repr(C, packed)]
pub struct ValidationEntry {
    /// Header ID (must be 1)
    pub header_id: u8,

    /// Platform ID
    pub platform_id: u8,

    /// Reserved (0)
    pub reserved: u16,

    /// Manufacturer/developer ID string (24 bytes)
    pub id_string: [u8; 24],

    /// Checksum word
    pub checksum: u16,

    /// Key bytes (0x55, 0xAA)
    pub key: [u8; 2],
}

impl ValidationEntry {
    /// Header ID constant
    pub const HEADER_ID: u8 = 0x01;

    /// Key bytes constant
    pub const KEY_BYTES: [u8; 2] = [0x55, 0xAA];

    /// Validate entry
    pub fn is_valid(&self) -> bool {
        self.header_id == Self::HEADER_ID && self.key == Self::KEY_BYTES && self.verify_checksum()
    }

    /// Verify checksum
    fn verify_checksum(&self) -> bool {
        // Convert struct to byte slice
        let bytes = unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, 32) };
        checksum::verify_checksum_16(bytes)
    }
}
