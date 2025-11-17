//! DOS Header structure and parsing

use super::super::{PeError, PeResult};
use super::utils::{read_u16, read_u32};

/// DOS Header (MZ header)
#[derive(Debug, Clone, Copy)]
pub struct DosHeader {
    pub e_magic: u16,  // "MZ" signature (0x5A4D)
    pub e_lfanew: u32, // Offset to PE header
}

impl DosHeader {
    pub const SIGNATURE: u16 = 0x5A4D; // "MZ"

    /// Parse DOS header from memory
    ///
    /// # Safety
    /// Caller must ensure data points to valid memory of at least 0x40 bytes
    pub unsafe fn parse(data: *const u8, size: usize) -> PeResult<Self> {
        if size < 0x40 {
            return Err(PeError::InvalidOffset);
        }

        let e_magic = read_u16(data, 0);
        if e_magic != Self::SIGNATURE {
            return Err(PeError::InvalidSignature);
        }

        let e_lfanew = read_u32(data, 0x3C);

        Ok(DosHeader { e_magic, e_lfanew })
    }
}
