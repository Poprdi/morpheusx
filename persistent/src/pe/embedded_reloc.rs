//! Parse embedded relocation metadata from .morpheus section
//! 
//! UEFI discards .reloc from memory after loading, so we embed a copy
//! in a custom section that survives in the loaded image.

use super::{PeError, PeResult};

const MORPHEUS_MAGIC: &[u8; 4] = b"MRPH";

/// Embedded relocation metadata
pub struct EmbeddedRelocData {
    pub original_rva: u32,
    pub data: &'static [u8],
}

/// Find and parse .morpheus section containing reloc metadata
/// 
/// # Safety
/// Caller must ensure image_base points to valid PE in memory
pub unsafe fn find_embedded_reloc(
    image_base: *const u8,
    image_size: usize,
) -> PeResult<EmbeddedRelocData> {
    // Parse DOS header
    if image_size < 0x40 {
        return Err(PeError::InvalidOffset);
    }
    
    let e_lfanew = u32::from_le_bytes([
        *image_base.add(0x3C),
        *image_base.add(0x3D),
        *image_base.add(0x3E),
        *image_base.add(0x3F),
    ]) as usize;
    
    // Get section count
    let section_count = u16::from_le_bytes([
        *image_base.add(e_lfanew + 6),
        *image_base.add(e_lfanew + 7),
    ]) as usize;
    
    let opt_header_size = u16::from_le_bytes([
        *image_base.add(e_lfanew + 20),
        *image_base.add(e_lfanew + 21),
    ]) as usize;
    
    let section_table_offset = e_lfanew + 24 + opt_header_size;
    
    // Find .morpheus section
    for i in 0..section_count {
        let sec_offset = section_table_offset + (i * 40);
        
        if sec_offset + 40 > image_size {
            break;
        }
        
        let name_ptr = image_base.add(sec_offset);
        let name = core::slice::from_raw_parts(name_ptr, 8);
        
        // Check if this is .morpheus section
        if name.starts_with(b".morphe") {
            let virtual_size = u32::from_le_bytes([
                *image_base.add(sec_offset + 8),
                *image_base.add(sec_offset + 9),
                *image_base.add(sec_offset + 10),
                *image_base.add(sec_offset + 11),
            ]);
            
            let virtual_address = u32::from_le_bytes([
                *image_base.add(sec_offset + 12),
                *image_base.add(sec_offset + 13),
                *image_base.add(sec_offset + 14),
                *image_base.add(sec_offset + 15),
            ]);
            
            if virtual_size < 12 {
                return Err(PeError::CorruptedData);
            }
            
            // Parse morpheus section data
            let morpheus_ptr = image_base.add(virtual_address as usize);
            
            // Check magic
            let magic = core::slice::from_raw_parts(morpheus_ptr, 4);
            if magic != MORPHEUS_MAGIC {
                return Err(PeError::CorruptedData);
            }
            
            // Read original RVA and size
            let original_rva = u32::from_le_bytes([
                *morpheus_ptr.add(4),
                *morpheus_ptr.add(5),
                *morpheus_ptr.add(6),
                *morpheus_ptr.add(7),
            ]);
            
            let data_size = u32::from_le_bytes([
                *morpheus_ptr.add(8),
                *morpheus_ptr.add(9),
                *morpheus_ptr.add(10),
                *morpheus_ptr.add(11),
            ]) as usize;
            
            if data_size > virtual_size as usize - 12 {
                return Err(PeError::CorruptedData);
            }
            
            // Extract reloc data
            let data = core::slice::from_raw_parts(morpheus_ptr.add(12), data_size);
            
            return Ok(EmbeddedRelocData {
                original_rva,
                data,
            });
        }
    }
    
    Err(PeError::MissingSection)
}
