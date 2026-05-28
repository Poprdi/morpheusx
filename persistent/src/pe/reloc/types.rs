//! PE base relocation table (PE/COFF §5.6). Format is platform-neutral,
//! application is per-arch.

use super::super::PeResult;

/// .reloc block header preceding `(block_size - 8) / 2` `u16` entries.
#[repr(C, packed)]
pub struct BaseRelocationBlock {
    pub page_rva: u32,
    /// Total block size including this 8-byte header.
    pub block_size: u32,
}

impl BaseRelocationBlock {
    pub const SIZE: usize = 8;

    pub fn entry_count(&self) -> usize {
        ((self.block_size as usize) - Self::SIZE) / 2
    }
}

/// Packed relocation: upper 4 bits = type, lower 12 = page offset.
#[derive(Debug, Clone, Copy)]
pub struct RelocationEntry {
    raw: u16,
}

impl RelocationEntry {
    pub fn new(raw: u16) -> Self {
        Self { raw }
    }

    pub fn reloc_type(&self) -> RelocationType {
        RelocationType::from_u16(self.raw >> 12)
    }

    pub fn offset(&self) -> u16 {
        self.raw & 0x0FFF
    }
}

/// IMAGE_REL_BASED_* codes (PE/COFF §5.6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum RelocationType {
    /// Padding; skip.
    Absolute = 0,
    High = 1,
    Low = 2,
    HighLow = 3,
    HighAdj = 4,
    // 5-9: arch-specific.
    /// 64-bit pointer fixup (x86_64, ARM64).
    Dir64 = 10,
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

        let entries_ptr = self.data[entries_start..].as_ptr() as *const u16;
        let entries = unsafe { core::slice::from_raw_parts(entries_ptr, entry_count) };

        self.offset += block_size as usize;

        Some((page_rva, block_size, entries))
    }
}

/// Per-arch fixup application. x86_64 = pointer add; ARM64 may also patch
/// ADRP/ADD immediates; ARM32 needs Thumb-mode handling.
pub trait RelocationEngine {
    /// Apply at UEFI load time (add delta).
    fn apply_relocation(
        &self,
        image_data: &mut [u8],
        entry: RelocationEntry,
        page_rva: u32,
        delta: i64,
    ) -> PeResult<()>;

    /// Reverse to recover the file-layout image (subtract delta).
    fn unapply_relocation(
        &self,
        image_data: &mut [u8],
        entry: RelocationEntry,
        page_rva: u32,
        delta: i64,
    ) -> PeResult<()>;

    fn arch(&self) -> super::super::PeArch;
}
