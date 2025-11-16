//! x86_64-specific relocation handling
//!
//! PE32+ format with IMAGE_REL_BASED_DIR64 relocations.
//! Simple pointer fixups - just add/subtract delta from 64-bit values.

use crate::pe::reloc::{RelocationEngine, RelocationEntry, RelocationType};
use crate::pe::{PeArch, PeError, PeResult};

/// x86_64 relocation engine
pub struct X64RelocationEngine;

impl RelocationEngine for X64RelocationEngine {
    fn apply_relocation(
        &self,
        image_data: &mut [u8],
        entry: RelocationEntry,
        page_rva: u32,
        delta: i64,
    ) -> PeResult<()> {
        // TODO: Implement x86_64 relocation application
        //
        // For DIR64 relocations:
        // 1. Calculate absolute RVA: page_rva + entry.offset()
        // 2. Read 64-bit value at that location
        // 3. Add delta: value = value + delta
        // 4. Write back
        //
        // x86_64 is straightforward - no instruction encoding tricks

        match entry.reloc_type() {
            RelocationType::Absolute => Ok(()), // Skip padding
            RelocationType::Dir64 => {
                // TODO: Implement DIR64 fixup
                todo!("Implement x86_64 DIR64 relocation")
            }
            _ => Err(PeError::UnsupportedFormat), // Unexpected type
        }
    }

    fn unapply_relocation(
        &self,
        image_data: &mut [u8],
        entry: RelocationEntry,
        page_rva: u32,
        delta: i64,
    ) -> PeResult<()> {
        // TODO: Implement x86_64 relocation reversal
        //
        // Same as apply but subtract instead of add:
        // value = value - delta
        //
        // This creates the original disk image from memory

        match entry.reloc_type() {
            RelocationType::Absolute => Ok(()),
            RelocationType::Dir64 => {
                // TODO: Implement DIR64 unfixup
                todo!("Implement x86_64 DIR64 unrelocate")
            }
            _ => Err(PeError::UnsupportedFormat),
        }
    }

    fn arch(&self) -> PeArch {
        PeArch::X64
    }
}

// Platform-specific notes:
//
// x86_64 UEFI uses PE32+ format (magic 0x20B)
// ImageBase is typically 0x400000 (linker default)
// UEFI loader picks actual load address (often different)
//
// Relocation delta = actual_load_address - original_ImageBase
//
// Example:
//   Original ImageBase: 0x0000000000400000
//   Actual load addr:   0x0000000076E4C000
//   Delta:              0x0000000076A4C000
//
// All DIR64 relocations get this delta added.
// To reverse: subtract the delta.
