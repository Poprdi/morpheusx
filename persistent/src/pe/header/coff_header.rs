//! COFF File Header structure and parsing

use super::super::{PeArch, PeError, PeResult};
use super::utils::{read_u16, read_u32};

/// COFF File Header
#[derive(Debug, Clone, Copy)]
pub struct CoffHeader {
    pub machine: u16,            // Target machine type
    pub number_of_sections: u16, // Number of sections
    pub time_date_stamp: u32,    // Timestamp
    pub size_of_optional_header: u16,
    pub characteristics: u16,
}

impl CoffHeader {
    // Machine types
    pub const MACHINE_AMD64: u16 = 0x8664; // x86_64
    pub const MACHINE_ARM64: u16 = 0xAA64; // aarch64
    pub const MACHINE_ARMNT: u16 = 0x01C4; // armv7 (Thumb-2)

    pub const PE_SIGNATURE: u32 = 0x00004550; // "PE\0\0"

    /// Parse COFF header from memory
    ///
    /// # Safety
    /// Caller must ensure data + pe_offset points to valid PE signature + COFF header
    pub unsafe fn parse(data: *const u8, pe_offset: u32, size: usize) -> PeResult<Self> {
        let offset = pe_offset as usize;

        if offset + 24 > size {
            return Err(PeError::InvalidOffset);
        }

        // Verify PE signature first
        let pe_sig = read_u32(data, offset);
        if pe_sig != Self::PE_SIGNATURE {
            return Err(PeError::InvalidSignature);
        }

        // COFF header starts at offset + 4
        let coff_offset = offset + 4;

        let machine = read_u16(data, coff_offset);
        let number_of_sections = read_u16(data, coff_offset + 2);
        let time_date_stamp = read_u32(data, coff_offset + 4);
        let size_of_optional_header = read_u16(data, coff_offset + 16);
        let characteristics = read_u16(data, coff_offset + 18);

        Ok(CoffHeader {
            machine,
            number_of_sections,
            time_date_stamp,
            size_of_optional_header,
            characteristics,
        })
    }

    /// Determine architecture from machine type
    pub fn arch(&self) -> PeResult<PeArch> {
        match self.machine {
            Self::MACHINE_AMD64 => Ok(PeArch::X64),
            Self::MACHINE_ARM64 => Ok(PeArch::ARM64),
            Self::MACHINE_ARMNT => Ok(PeArch::ARM),
            _ => Err(PeError::UnsupportedFormat),
        }
    }

    /// Get machine name
    pub fn machine_name(&self) -> &'static str {
        match self.machine {
            Self::MACHINE_AMD64 => "x86_64 (AMD64)",
            Self::MACHINE_ARM64 => "aarch64 (ARM64)",
            Self::MACHINE_ARMNT => "armv7 (Thumb-2)",
            _ => "Unknown",
        }
    }
}
