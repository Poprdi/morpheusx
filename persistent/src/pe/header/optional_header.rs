//! PE Optional Header (PE32+ / 64-bit) structure and parsing

use super::super::{PeError, PeResult};
use super::utils::{read_u16, read_u32, read_u64};

/// PE Optional Header (PE32+ / 64-bit)
#[derive(Debug, Clone, Copy)]
pub struct OptionalHeader64 {
    pub magic: u16, // 0x20B for PE32+
    pub address_of_entry_point: u32,
    pub image_base: u64, // Load address (UEFI modifies this!)
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub checksum: u32,
    pub subsystem: u16,
    pub number_of_rva_and_sizes: u32,
}

impl OptionalHeader64 {
    pub const MAGIC_PE32PLUS: u16 = 0x20B;
    pub const IMAGE_BASE_OFFSET: usize = 24;

    /// Parse optional header from memory
    ///
    /// # Safety
    /// Caller must ensure data + offset points to valid optional header
    pub unsafe fn parse(data: *const u8, pe_offset: u32, size: usize) -> PeResult<Self> {
        // Optional header starts at: PE offset + 4 (sig) + 20 (COFF)
        let opt_offset = pe_offset as usize + 24;

        if opt_offset + 96 > size {
            return Err(PeError::InvalidOffset);
        }

        let magic = read_u16(data, opt_offset);
        if magic != Self::MAGIC_PE32PLUS {
            return Err(PeError::UnsupportedFormat);
        }

        let address_of_entry_point = read_u32(data, opt_offset + 16);
        let image_base = read_u64(data, opt_offset + 24);
        let section_alignment = read_u32(data, opt_offset + 32);
        let file_alignment = read_u32(data, opt_offset + 36);
        let size_of_image = read_u32(data, opt_offset + 56);
        let size_of_headers = read_u32(data, opt_offset + 60);
        let checksum = read_u32(data, opt_offset + 64);
        let subsystem = read_u16(data, opt_offset + 68);
        let number_of_rva_and_sizes = read_u32(data, opt_offset + 108);

        Ok(OptionalHeader64 {
            magic,
            address_of_entry_point,
            image_base,
            section_alignment,
            file_alignment,
            size_of_image,
            size_of_headers,
            checksum,
            subsystem,
            number_of_rva_and_sizes,
        })
    }

    /// Patch ImageBase field in a buffer
    ///
    /// # Safety
    /// Caller must ensure data is valid PE with proper DOS/COFF headers
    pub unsafe fn patch_image_base(data: &mut [u8], new_image_base: u64) -> PeResult<()> {
        if data.len() < 0x40 {
            return Err(PeError::InvalidOffset);
        }

        // Read e_lfanew to find PE header
        let e_lfanew =
            u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]]) as usize;

        // ImageBase is at: PE offset + 4 (sig) + 20 (COFF) + 24
        let image_base_offset = e_lfanew + 24 + Self::IMAGE_BASE_OFFSET;

        if image_base_offset + 8 > data.len() {
            return Err(PeError::InvalidOffset);
        }

        // Write new ImageBase
        let bytes = new_image_base.to_le_bytes();
        data[image_base_offset..image_base_offset + 8].copy_from_slice(&bytes);

        Ok(())
    }
}
