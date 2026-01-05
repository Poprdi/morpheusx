//! ESP (EFI System Partition) persistence backend (Future API)
//!
//! This module defines a trait-based wrapper around FAT32 operations.
//!
//! # Current Status
//!
//! This API is **not yet implemented**. The current working implementation
//! uses `morpheus_core::fs::fat32_ops::write_file()` directly in the
//! bootloader installer at `bootloader/src/installer/operations.rs`.
//!
//! # Future Usage
//!
//! ```ignore
//! let mut esp = EspBackend::new(adapter, esp_start_lba);
//! esp.store_bootloader(&bootable_image)?;
//! ```

use super::PersistenceBackend;
use crate::pe::PeError;

/// ESP/FAT32 persistence backend (Layer 0)
///
/// Primary bootable storage - writes to `/EFI/BOOT/BOOTX64.EFI` on the ESP.
pub struct EspBackend {
    // Future fields:
    // - block_io: Block I/O adapter  
    // - partition_lba: Start LBA of ESP partition
    // - path: Path to bootloader file
    _private: (),
}

impl EspBackend {
    /// Create ESP backend for a specific partition
    ///
    /// # Note
    /// Not yet implemented. Use `fat32_ops::write_file()` directly for now.
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl PersistenceBackend for EspBackend {
    fn store_bootloader(&mut self, _data: &[u8]) -> Result<(), PeError> {
        // Future: Use morpheus_core::fs::fat32_ops::write_file
        unimplemented!("Use fat32_ops::write_file() directly for now")
    }

    fn retrieve_bootloader(&mut self) -> Result<alloc::vec::Vec<u8>, PeError> {
        // Future: Use morpheus_core::fs::fat32_ops::read_file
        unimplemented!("Use fat32_ops::read_file() directly for now")
    }

    fn is_persisted(&mut self) -> Result<bool, PeError> {
        // Future: Use morpheus_core::fs::fat32_ops::file_exists
        unimplemented!("Use fat32_ops::file_exists() directly for now")
    }

    fn name(&self) -> &str {
        "ESP (Layer 0)"
    }
}
