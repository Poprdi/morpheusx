//! ESP (EFI System Partition) persistence backend — unimplemented.
//!
//! Bootloader currently writes via `morpheus_storage_format::fs::fat32_ops`
//! directly (`bootloader/src/installer/operations.rs`); this trait wrapper is
//! a placeholder.

use super::PersistenceBackend;
use crate::pe::PeError;

/// ESP/FAT32 backend: writes `/EFI/BOOT/BOOTX64.EFI` on the ESP.
pub struct EspBackend {
    _private: (),
}

impl EspBackend {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl PersistenceBackend for EspBackend {
    fn store_bootloader(&mut self, _data: &[u8]) -> Result<(), PeError> {
        // Future: Use morpheus_storage_format::fs::fat32_ops::write_file
        unimplemented!("Use fat32_ops::write_file() directly for now")
    }

    fn retrieve_bootloader(&mut self) -> Result<alloc::vec::Vec<u8>, PeError> {
        // Future: Use morpheus_storage_format::fs::fat32_ops::read_file
        unimplemented!("Use fat32_ops::read_file() directly for now")
    }

    fn is_persisted(&mut self) -> Result<bool, PeError> {
        // Future: Use morpheus_storage_format::fs::fat32_ops::file_exists
        unimplemented!("Use fat32_ops::file_exists() directly for now")
    }

    fn name(&self) -> &str {
        "ESP (Layer 0)"
    }
}
