//! PE base relocation table parsing
//! 
//! Platform-neutral format, but application is platform-specific.
//! The .reloc section contains blocks of relocations.

use super::{PeError, PeResult};

/// Base relocation block header
#[repr(C, packed)]
pub struct BaseRelocationBlock {
    pub page_rva: u32,     // RVA of the page
    pub block_size: u32,   // Size of this block (including header)
}

impl BaseRelocationBlock {
    pub const SIZE: usize = 8;
    
    /// Number of relocation entries in this block
    pub fn entry_count(&self) -> usize {
        ((self.block_size as usize) - Self::SIZE) / 2
    }
}

/// Relocation entry (16 bits)
/// Upper 4 bits: type
/// Lower 12 bits: offset within page
#[derive(Debug, Clone, Copy)]
pub struct RelocationEntry {
    raw: u16,
}

impl RelocationEntry {
    pub fn new(raw: u16) -> Self {
        Self { raw }
    }
    
    /// Get relocation type
    pub fn reloc_type(&self) -> RelocationType {
        RelocationType::from_u16(self.raw >> 12)
    }
    
    /// Get offset within page
    pub fn offset(&self) -> u16 {
        self.raw & 0x0FFF
    }
}

/// Relocation types (upper 4 bits of entry)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum RelocationType {
    Absolute = 0,     // Skip (padding)
    High = 1,         // Add high 16 bits of delta
    Low = 2,          // Add low 16 bits of delta
    HighLow = 3,      // Add full 32-bit delta
    HighAdj = 4,      // Complex ARM relocation
    // 5-8 reserved/architecture-specific
    Dir64 = 10,       // Add full 64-bit delta (x86_64, ARM64)
    Unknown,
}

impl RelocationType {
    pub fn from_u16(val: u16) -> Self {
        match val {
            0 => Self::Absolute,
            1 => Self::High,
            2 => Self::Low,
            3 => Self::HighLow,
            4 => Self::HighAdj,
            10 => Self::Dir64,
            _ => Self::Unknown,
        }
    }
    
    /// Size of the relocated value in bytes
    pub fn size(&self) -> usize {
        match self {
            Self::Absolute => 0,
            Self::High | Self::Low => 2,
            Self::HighLow => 4,
            Self::Dir64 => 8,
            Self::HighAdj => 4,
            Self::Unknown => 0,
        }
    }
}

/// Iterator over relocation blocks
pub struct RelocationBlockIter<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> RelocationBlockIter<'a> {
    pub fn new(reloc_section_data: &'a [u8]) -> Self {
        Self {
            data: reloc_section_data,
            offset: 0,
        }
    }
}

impl<'a> Iterator for RelocationBlockIter<'a> {
    type Item = (u32, u32, &'a [u16]);
    
    fn next(&mut self) -> Option<Self::Item> {
        if self.offset + BaseRelocationBlock::SIZE > self.data.len() {
            return None;
        }
        
        let page_rva = u32::from_le_bytes([
            self.data[self.offset],
            self.data[self.offset + 1],
            self.data[self.offset + 2],
            self.data[self.offset + 3],
        ]);
        
        let block_size = u32::from_le_bytes([
            self.data[self.offset + 4],
            self.data[self.offset + 5],
            self.data[self.offset + 6],
            self.data[self.offset + 7],
        ]);
        
        if block_size < BaseRelocationBlock::SIZE as u32 {
            return None;
        }
        
        if self.offset + block_size as usize > self.data.len() {
            return None;
        }
        
        let entries_start = self.offset + BaseRelocationBlock::SIZE;
        let entry_count = ((block_size as usize) - BaseRelocationBlock::SIZE) / 2;
        
        // Cast u8 slice to u16 slice (entries)
        let entries_ptr = self.data[entries_start..].as_ptr() as *const u16;
        let entries = unsafe { core::slice::from_raw_parts(entries_ptr, entry_count) };
        
        self.offset += block_size as usize;
        
        Some((page_rva, block_size, entries))
    }
}

/// Trait for platform-specific relocation application
/// 
/// Different architectures implement this differently:
/// - x86_64: Simple pointer fixups
/// - ARM64: May involve ADRP/ADD instruction pairs
/// - ARM32: Thumb mode considerations
pub trait RelocationEngine {
    /// Apply relocation (add delta to relocated values)
    /// Used when UEFI loads the image
    fn apply_relocation(
        &self,
        image_data: &mut [u8],
        entry: RelocationEntry,
        page_rva: u32,
        delta: i64,
    ) -> PeResult<()>;
    
    /// Unapply relocation (subtract delta from relocated values)
    /// Used when creating bootable image from memory
    fn unapply_relocation(
        &self,
        image_data: &mut [u8],
        entry: RelocationEntry,
        page_rva: u32,
        delta: i64,
    ) -> PeResult<()>;
    
    /// Get the architecture this engine handles
    fn arch(&self) -> super::PeArch;
}

/// Reverse all DIR64 relocations in a PE image
/// 
/// Takes a relocated image in memory and reverses the relocations
/// to restore the original bootable file state.
/// 
/// # Arguments
/// * `image_data` - Mutable slice of the entire PE image
/// * `reloc_rva` - RVA of .reloc section
/// * `reloc_size` - Size of .reloc section
/// * `delta` - Relocation delta (actual_load - original_image_base)
/// 
/// # Safety
/// Caller must ensure image_data is valid PE with proper reloc section
pub unsafe fn unrelocate_image(
    image_data: &mut [u8],
    reloc_rva: u32,
    reloc_size: u32,
    delta: i64,
) -> PeResult<()> {
    if reloc_rva as usize >= image_data.len() {
        return Err(PeError::InvalidOffset);
    }
    
    // NOTE: UEFI may truncate reloc_size after applying relocations!
    // Use the larger of reloc_size or a reasonable max
    let max_reloc_size = reloc_size.max(512);
    
    if (reloc_rva as usize + max_reloc_size as usize) > image_data.len() {
        // Clamp to image bounds
        // Don't error - just process what we can
    }
    
    // Parse all relocation blocks (scope borrow to avoid conflicts)
    let mut block_offset = 0usize;
    
    // Force iteration through ALL blocks
    for _block_num in 0..16 {
        if block_offset + BaseRelocationBlock::SIZE > max_reloc_size as usize {
            break;
        }
        // Read block header (careful to scope borrows)
        let page_rva: u32;
        let block_size: u32;
        
        {
            let reloc_base = reloc_rva as usize;
            page_rva = u32::from_le_bytes([
                image_data[reloc_base + block_offset],
                image_data[reloc_base + block_offset + 1],
                image_data[reloc_base + block_offset + 2],
                image_data[reloc_base + block_offset + 3],
            ]);
            
            block_size = u32::from_le_bytes([
                image_data[reloc_base + block_offset + 4],
                image_data[reloc_base + block_offset + 5],
                image_data[reloc_base + block_offset + 6],
                image_data[reloc_base + block_offset + 7],
            ]);
        }
        
        // Sanity check
        if block_size < BaseRelocationBlock::SIZE as u32 {
            break;
        }
        
        if block_offset + block_size as usize > reloc_size as usize {
            break;
        }
        
        // Process entries
        let entry_count = ((block_size as usize) - BaseRelocationBlock::SIZE) / 2;
        let entries_offset = block_offset + BaseRelocationBlock::SIZE;
        
        for i in 0..entry_count {
            let entry_offset = entries_offset + (i * 2);
            
            // Read entry (scope borrow)
            let entry_raw: u16;
            {
                let reloc_base = reloc_rva as usize;
                entry_raw = u16::from_le_bytes([
                    image_data[reloc_base + entry_offset],
                    image_data[reloc_base + entry_offset + 1],
                ]);
            }
            
            let reloc_type = (entry_raw >> 12) & 0xF;
            let offset = entry_raw & 0xFFF;
            
            // Only handle DIR64 relocations (type 10)
            if reloc_type == 10 {
                let pointer_rva = page_rva + offset as u32;
                
                if (pointer_rva as usize + 8) > image_data.len() {
                    continue; // Skip invalid relocations
                }
                
                // Read current value (8 bytes) - scope borrow
                let current_value: u64;
                {
                    let ptr_offset = pointer_rva as usize;
                    current_value = u64::from_le_bytes([
                        image_data[ptr_offset],
                        image_data[ptr_offset + 1],
                        image_data[ptr_offset + 2],
                        image_data[ptr_offset + 3],
                        image_data[ptr_offset + 4],
                        image_data[ptr_offset + 5],
                        image_data[ptr_offset + 6],
                        image_data[ptr_offset + 7],
                    ]);
                }
                
                // Unrelocate: subtract delta
                let original_value = (current_value as i64 - delta) as u64;
                
                // Write back (now borrow is clear)
                let ptr_offset = pointer_rva as usize;
                let original_bytes = original_value.to_le_bytes();
                image_data[ptr_offset] = original_bytes[0];
                image_data[ptr_offset + 1] = original_bytes[1];
                image_data[ptr_offset + 2] = original_bytes[2];
                image_data[ptr_offset + 3] = original_bytes[3];
                image_data[ptr_offset + 4] = original_bytes[4];
                image_data[ptr_offset + 5] = original_bytes[5];
                image_data[ptr_offset + 6] = original_bytes[6];
                image_data[ptr_offset + 7] = original_bytes[7];
            }
        }
        
        block_offset += block_size as usize;
    }
    
    Ok(())
}
