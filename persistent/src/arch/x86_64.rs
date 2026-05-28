//! PE32+ IMAGE_REL_BASED_DIR64 fixups: add/subtract delta on 64-bit values.

use crate::pe::reloc::{RelocationEngine, RelocationEntry, RelocationType};
use crate::pe::{PeArch, PeError, PeResult};

pub struct X64RelocationEngine;

impl RelocationEngine for X64RelocationEngine {
    fn apply_relocation(
        &self,
        image_data: &mut [u8],
        entry: RelocationEntry,
        page_rva: u32,
        delta: i64,
    ) -> PeResult<()> {
        match entry.reloc_type() {
            RelocationType::Absolute => Ok(()), // padding
            RelocationType::Dir64 => {
                let rva = page_rva as usize + entry.offset() as usize;
                if rva + 8 > image_data.len() {
                    return Err(PeError::InvalidOffset);
                }

                let current = u64::from_le_bytes([
                    image_data[rva],
                    image_data[rva + 1],
                    image_data[rva + 2],
                    image_data[rva + 3],
                    image_data[rva + 4],
                    image_data[rva + 5],
                    image_data[rva + 6],
                    image_data[rva + 7],
                ]);

                let relocated = (current as i64 + delta) as u64;
                let bytes = relocated.to_le_bytes();
                image_data[rva..rva + 8].copy_from_slice(&bytes);

                Ok(())
            },
            _ => Err(PeError::UnsupportedFormat),
        }
    }

    fn unapply_relocation(
        &self,
        image_data: &mut [u8],
        entry: RelocationEntry,
        page_rva: u32,
        delta: i64,
    ) -> PeResult<()> {
        match entry.reloc_type() {
            RelocationType::Absolute => Ok(()),
            RelocationType::Dir64 => {
                let rva = page_rva as usize + entry.offset() as usize;
                if rva + 8 > image_data.len() {
                    return Err(PeError::InvalidOffset);
                }

                let current = u64::from_le_bytes([
                    image_data[rva],
                    image_data[rva + 1],
                    image_data[rva + 2],
                    image_data[rva + 3],
                    image_data[rva + 4],
                    image_data[rva + 5],
                    image_data[rva + 6],
                    image_data[rva + 7],
                ]);

                let original = (current as i64 - delta) as u64;
                let bytes = original.to_le_bytes();
                image_data[rva..rva + 8].copy_from_slice(&bytes);

                Ok(())
            },
            _ => Err(PeError::UnsupportedFormat),
        }
    }

    fn arch(&self) -> PeArch {
        PeArch::X64
    }
}

// PE32+ magic 0x20B. ImageBase typically 0x400000; UEFI picks actual load addr.
// delta = actual_load_addr - ImageBase; all DIR64 entries get delta added.
