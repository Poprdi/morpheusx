//! Memory image capture (Future API)
//!
//! This module defines a higher-level API for capturing and unrelocating PE images.
//!
//! # Current Status
//!
//! This API is **not yet implemented**. The current working implementation uses:
//! - `PeHeaders::unrelocate_image()` in `pe/header/pe_headers.rs`
//! - `unrelocate_image()` in `pe/reloc/unrelocate.rs`
//! - Direct integration in `bootloader/src/installer/operations.rs`
//!
//! This module exists as a future abstraction layer that would provide a cleaner API.
//!
//! # Future Usage
//!
//! ```ignore
//! let captured = MemoryImage::capture_from_memory(image_base, image_size)?;
//! let bootable = captured.create_bootable_image()?;
//! esp_backend.store_bootloader(&bootable)?;
//! ```

use crate::pe::PeError;

/// Captured memory image of running bootloader
///
/// This struct holds a captured PE image along with metadata needed
/// to reverse relocations and create a bootable disk image.
pub struct MemoryImage {
    /// Raw image data (as loaded by UEFI)
    pub data: alloc::vec::Vec<u8>,

    /// Base address where image is loaded
    pub load_address: u64,

    /// Original ImageBase from PE header (before UEFI modified it)
    pub original_image_base: u64,

    /// Relocation delta (load_address - original_image_base)
    pub relocation_delta: i64,
}

impl MemoryImage {
    /// Capture running bootloader from UEFI LoadedImage protocol
    ///
    /// # Arguments
    /// * `image_base` - Pointer to loaded image (from LoadedImageProtocol.image_base)
    /// * `image_size` - Size of loaded image (from LoadedImageProtocol.image_size)
    ///
    /// # Returns
    /// Captured image with relocation information
    ///
    /// # Note
    /// Not yet implemented. See `bootloader/src/installer/operations.rs` for
    /// the current working implementation.
    pub fn capture_from_memory(
        _image_base: *const u8,
        _image_size: usize,
    ) -> Result<Self, PeError> {
        // Future implementation would:
        // 1. Copy image data to Vec
        // 2. Parse PE headers
        // 3. Reconstruct original ImageBase
        // 4. Calculate relocation delta
        unimplemented!("Use PeHeaders::unrelocate_image() directly for now")
    }

    /// Create bootable disk image by reversing relocations
    ///
    /// # Note
    /// Not yet implemented. See `PeHeaders::unrelocate_image()` and
    /// `PeHeaders::rva_to_file_layout()` for the current working implementation.
    pub fn create_bootable_image(&self) -> Result<alloc::vec::Vec<u8>, PeError> {
        // Future implementation would use the RelocationEngine trait
        unimplemented!(
            "Use PeHeaders::unrelocate_image() and rva_to_file_layout() directly for now"
        )
    }
}
