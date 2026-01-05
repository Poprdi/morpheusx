//! x86_64-specific relocation handling
//!
//! PE32+ format with IMAGE_REL_BASED_DIR64 relocations.
//! Simple pointer fixups - just add/subtract delta from 64-bit values.

use crate::pe::reloc::{RelocationEngine, RelocationEntry, RelocationType};
use crate::pe::{PeArch, PeError, PeResult};

/// x86_64 relocation engine
///
/// Implements the `RelocationEngine` trait for x86_64 PE32+ binaries.
/// Uses simple 64-bit pointer fixups (no instruction encoding required).
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
            RelocationType::Absolute => Ok(()), // Skip padding entries
            RelocationType::Dir64 => {
                let rva = page_rva as usize + entry.offset() as usize;
                if rva + 8 > image_data.len() {
                    return Err(PeError::InvalidOffset);
                }
                
                // Read current 64-bit value
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
                
                // Apply relocation: add delta
                let relocated = (current as i64 + delta) as u64;
                let bytes = relocated.to_le_bytes();
                image_data[rva..rva + 8].copy_from_slice(&bytes);
                
                Ok(())
            }
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
            RelocationType::Absolute => Ok(()), // Skip padding entries
            RelocationType::Dir64 => {
                let rva = page_rva as usize + entry.offset() as usize;
                if rva + 8 > image_data.len() {
                    return Err(PeError::InvalidOffset);
                }
                
                // Read current 64-bit value
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
                
                // Unapply relocation: subtract delta
                let original = (current as i64 - delta) as u64;
                let bytes = original.to_le_bytes();
                image_data[rva..rva + 8].copy_from_slice(&bytes);
                
                Ok(())
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
