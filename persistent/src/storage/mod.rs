//! Persistence storage backends (Future API)
//!
//! This module defines a trait-based abstraction for multiple persistence layers.
//!
//! # Current Status
//!
//! This API is **not yet implemented**. The current working implementation uses
//! `morpheus_core::fs::fat32_ops::write_file()` directly in the bootloader installer.
//!
//! This module exists as a future abstraction for multi-layer persistence:
//! - Layer 0: ESP/FAT32 (primary bootable storage)
//! - Layer 1: TPM (cryptographic attestation)
//! - Layer 2: CMOS/NVRAM (emergency recovery stub)
//! - Layer 3: HVRAM (hypervisor-hidden persistence)

pub mod esp;

use crate::pe::PeError;

/// Trait for persistence backends
///
/// Different backends store the bootloader image in different ways.
/// The trait provides a unified interface for multi-layer persistence.
pub trait PersistenceBackend {
    /// Store bootloader image
    fn store_bootloader(&mut self, data: &[u8]) -> Result<(), PeError>;

    /// Retrieve bootloader image (for verification)
    fn retrieve_bootloader(&mut self) -> Result<alloc::vec::Vec<u8>, PeError>;

    /// Check if bootloader is already persisted
    fn is_persisted(&mut self) -> Result<bool, PeError>;

    /// Backend name for logging
    fn name(&self) -> &str;
}
