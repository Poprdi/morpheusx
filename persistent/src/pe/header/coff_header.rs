//! COFF File Header (PE/COFF §3.3).

use super::super::{PeArch, PeError, PeResult};
use super::utils::{read_u16, read_u32};

#[derive(Debug, Clone, Copy)]
pub struct CoffHeader {
    pub machine: u16,
    pub number_of_sections: u16,
    pub time_date_stamp: u32,
    pub size_of_optional_header: u16,
    pub characteristics: u16,
}

impl CoffHeader {
    pub const MACHINE_AMD64: u16 = 0x8664;
    pub const MACHINE_ARM64: u16 = 0xAA64;
    pub const MACHINE_ARMNT: u16 = 0x01C4;

    /// "PE\0\0".
    pub const PE_SIGNATURE: u32 = 0x00004550;

    /// # Safety
    ///
    /// `data` must be readable for at least `size` bytes, and
    /// `data + pe_offset` must point at the PE signature followed by a COFF
    /// header that lies entirely within `size`.
    pub unsafe fn parse(data: *const u8, pe_offset: u32, size: usize) -> PeResult<Self> {
        let offset = pe_offset as usize;

        if offset + 24 > size {
            return Err(PeError::InvalidOffset);
        }

        let pe_sig = read_u32(data, offset);
        if pe_sig != Self::PE_SIGNATURE {
            return Err(PeError::InvalidSignature);
        }

        // COFF header starts after the 4-byte signature.
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

    pub fn arch(&self) -> PeResult<PeArch> {
        match self.machine {
            Self::MACHINE_AMD64 => Ok(PeArch::X64),
            Self::MACHINE_ARM64 => Ok(PeArch::ARM64),
            Self::MACHINE_ARMNT => Ok(PeArch::ARM),
            _ => Err(PeError::UnsupportedFormat),
        }
    }

    pub fn machine_name(&self) -> &'static str {
        match self.machine {
            Self::MACHINE_AMD64 => "x86_64 (AMD64)",
            Self::MACHINE_ARM64 => "aarch64 (ARM64)",
            Self::MACHINE_ARMNT => "armv7 (Thumb-2)",
            _ => "Unknown",
        }
    }
}
