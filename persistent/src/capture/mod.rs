//! Placeholder higher-level API for capture + unrelocate. The working path is
//! `PeHeaders::unrelocate_image()` (pe/header/pe_headers.rs) called directly
//! from `bootloader/src/installer/operations.rs`.

use crate::pe::PeError;

pub struct MemoryImage {
    pub data: alloc::vec::Vec<u8>,
    pub load_address: u64,
    /// ImageBase from the PE optional header before UEFI applied fixups.
    pub original_image_base: u64,
    /// `load_address - original_image_base`.
    pub relocation_delta: i64,
}

impl MemoryImage {
    pub fn capture_from_memory(
        _image_base: *const u8,
        _image_size: usize,
    ) -> Result<Self, PeError> {
        unimplemented!("Use PeHeaders::unrelocate_image() directly for now")
    }

    pub fn create_bootable_image(&self) -> Result<alloc::vec::Vec<u8>, PeError> {
        unimplemented!(
            "Use PeHeaders::unrelocate_image() and rva_to_file_layout() directly for now"
        )
    }
}
