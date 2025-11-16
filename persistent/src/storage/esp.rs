//! ESP (EFI System Partition) persistence backend
//!
//! Uses existing FAT32 implementation from morpheus-core.
//! This is Layer 0 - primary bootable storage.

use super::PersistenceBackend;
use crate::pe::PeError;

pub struct EspBackend {
    // TODO: Add fields for ESP access
    // - Block I/O adapter
    // - Partition LBA
    // - Path to store bootloader
}

impl EspBackend {
    /// Create ESP backend for a specific partition
    pub fn new(/* TODO: parameters */) -> Self {
        todo!("Implement ESP backend creation")
    }
}

impl PersistenceBackend for EspBackend {
    fn store_bootloader(&mut self, data: &[u8]) -> Result<(), PeError> {
        // TODO: Use morpheus_core::fs::fat32_ops::write_file
        // Path: /EFI/BOOT/BOOTX64.EFI (or BOOTAA64.EFI for ARM)

        todo!("Implement ESP bootloader storage")
    }

    fn retrieve_bootloader(&mut self) -> Result<alloc::vec::Vec<u8>, PeError> {
        // TODO: Read bootloader file from ESP
        // Used for verification after installation

        todo!("Implement ESP bootloader retrieval")
    }

    fn is_persisted(&mut self) -> Result<bool, PeError> {
        // TODO: Use morpheus_core::fs::fat32_ops::file_exists

        todo!("Implement ESP persistence check")
    }

    fn name(&self) -> &str {
        "ESP (Layer 0)"
    }
}

// Integration point with existing installer:
//
// Old code in bootloader/src/installer/mod.rs:
//   fat32_ops::write_file(&mut adapter, esp.start_lba, "/EFI/BOOT/BOOTX64.EFI", &binary_data)
//
// New code:
//   let mut esp_backend = EspBackend::new(adapter, esp.start_lba);
//   let bootable_image = captured.create_bootable_image()?;
//   esp_backend.store_bootloader(&bootable_image)?;
//
// This abstracts the storage mechanism and prepares for multi-layer persistence.
