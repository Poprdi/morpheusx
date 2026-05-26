//! ARM64 PE32+ relocations. Stub: only DIR64 data fixups planned; ADRP/ADD
//! instruction patching deferred until we encounter binaries that need it.

use crate::pe::reloc::{RelocationEngine, RelocationEntry, RelocationType};
use crate::pe::{PeArch, PeError, PeResult};

pub struct Aarch64RelocationEngine;

impl RelocationEngine for Aarch64RelocationEngine {
    fn apply_relocation(
        &self,
        image_data: &mut [u8],
        entry: RelocationEntry,
        page_rva: u32,
        delta: i64,
    ) -> PeResult<()> {
        match entry.reloc_type() {
            RelocationType::Absolute => Ok(()),
            RelocationType::Dir64 => {
                todo!("ARM64 DIR64 relocation: detect ADRP/ADD vs data pointer")
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
            RelocationType::Dir64 => todo!("ARM64 DIR64 unrelocate"),
            _ => Err(PeError::UnsupportedFormat),
        }
    }

    fn arch(&self) -> PeArch {
        PeArch::ARM64
    }
}

// Machine type 0xAA64. ADRP/ADD PIC sequences may show up as DIR64 reloc
// targets; detect via opcode and patch the immediate, not a 64-bit pointer.
