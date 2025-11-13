//! Persistence storage backends
//! 
//! Multiple layers of persistence for bootloader and data.

pub mod esp;      // ESP/FAT32 storage (primary)
// Future layers:
// pub mod tpm;   // TPM PCR measurements
// pub mod cmos;  // CMOS/NVRAM micro-persistence
// pub mod hvram; // Hypervisor RAM persistence

use crate::pe::PeError;

/// Trait for persistence backends
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

// Layer 0: ESP Persistence (what works now)
// Layer 1: TPM attestation (cryptographic proof of boot state)
// Layer 2: CMOS/NVRAM (tiny stub for emergency recovery)
// Layer 3: HVRAM (if running virtualized, hide in hypervisor)
// 
// Each layer serves different purposes:
// - ESP: Primary bootable storage
// - TPM: Tamper detection
// - CMOS: Recovery if ESP corrupted
// - HVRAM: Stealth/anti-forensics
