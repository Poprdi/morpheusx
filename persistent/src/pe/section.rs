//! PE section table (PE/COFF §3).

use super::{PeError, PeResult};

/// Section header, 40 bytes (PE/COFF §3.1).
#[repr(C, packed)]
pub struct SectionHeader {
    /// Null-padded ASCII name.
    pub name: [u8; 8],
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub size_of_raw_data: u32,
    pub pointer_to_raw_data: u32,
    pub pointer_to_relocations: u32,
    pub pointer_to_linenumbers: u32,
    pub number_of_relocations: u16,
    pub number_of_linenumbers: u16,
    pub characteristics: u32,
}

impl SectionHeader {
    pub fn is_reloc_section(&self) -> bool {
        &self.name[..6] == b".reloc"
    }

    pub fn is_text_section(&self) -> bool {
        &self.name[..5] == b".text"
    }

    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&c| c == 0).unwrap_or(8);
        core::str::from_utf8(&self.name[..len]).unwrap_or("<invalid>")
    }
}

pub struct SectionTable<'a> {
    sections: &'a [SectionHeader],
}

impl<'a> SectionTable<'a> {
    /// SAFETY: `data + offset` must point to `count` valid `SectionHeader`s
    /// and lie within `image_size`.
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

        let sections = core::slice::from_raw_parts(data.add(offset) as *const SectionHeader, count);

        Ok(SectionTable { sections })
    }

    pub fn find_reloc_section(&self) -> Option<&SectionHeader> {
        self.sections.iter().find(|s| s.is_reloc_section())
    }

    pub fn get(&self, index: usize) -> Option<&SectionHeader> {
        self.sections.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SectionHeader> {
        self.sections.iter()
    }

    pub fn count(&self) -> usize {
        self.sections.len()
    }
}
