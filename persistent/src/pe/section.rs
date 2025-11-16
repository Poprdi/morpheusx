//! PE section table parsing
//!
//! Sections contain code, data, resources, and relocations.
//! The .reloc section is what we care about most.

use super::{PeError, PeResult};

/// Section header (40 bytes)
#[repr(C, packed)]
pub struct SectionHeader {
    pub name: [u8; 8],            // Section name (null-padded ASCII)
    pub virtual_size: u32,        // Size in memory
    pub virtual_address: u32,     // RVA in memory
    pub size_of_raw_data: u32,    // Size on disk
    pub pointer_to_raw_data: u32, // Offset on disk
    pub pointer_to_relocations: u32,
    pub pointer_to_linenumbers: u32,
    pub number_of_relocations: u16,
    pub number_of_linenumbers: u16,
    pub characteristics: u32,
}

impl SectionHeader {
    /// Check if this is the .reloc section
    pub fn is_reloc_section(&self) -> bool {
        &self.name[..6] == b".reloc"
    }

    /// Check if this is the .text section (code)
    pub fn is_text_section(&self) -> bool {
        &self.name[..5] == b".text"
    }

    /// Get section name as string (may contain non-UTF8)
    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&c| c == 0).unwrap_or(8);
        core::str::from_utf8(&self.name[..len]).unwrap_or("<invalid>")
    }
}

/// Section table - array of section headers
pub struct SectionTable<'a> {
    sections: &'a [SectionHeader],
}

impl<'a> SectionTable<'a> {
    /// Parse section table from PE data
    ///
    /// # Safety
    /// Caller must ensure data+offset points to valid section headers
    pub unsafe fn parse(
        data: *const u8,
        offset: usize,
        count: usize,
        image_size: usize,
    ) -> PeResult<Self> {
        if count == 0 {
            return Err(PeError::InvalidOffset);
        }

        let table_size = count * core::mem::size_of::<SectionHeader>();
        if offset + table_size > image_size {
            return Err(PeError::InvalidOffset);
        }

        // Cast raw pointer to slice of SectionHeaders
        let sections = core::slice::from_raw_parts(data.add(offset) as *const SectionHeader, count);

        Ok(SectionTable { sections })
    }

    /// Find the .reloc section
    pub fn find_reloc_section(&self) -> Option<&SectionHeader> {
        self.sections.iter().find(|s| s.is_reloc_section())
    }

    /// Get section by index
    pub fn get(&self, index: usize) -> Option<&SectionHeader> {
        self.sections.get(index)
    }

    /// Iterator over all sections
    pub fn iter(&self) -> impl Iterator<Item = &SectionHeader> {
        self.sections.iter()
    }

    /// Number of sections
    pub fn count(&self) -> usize {
        self.sections.len()
    }
}
