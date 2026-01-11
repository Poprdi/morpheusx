//! Boot catalog parsing
//!
//! El Torito Boot Catalog structure and parsing.

use super::entry::BootEntry;
use super::validation::ValidationEntry;
use crate::error::{Iso9660Error, Result};

/// Boot Catalog
///
/// The boot catalog starts with a validation entry followed by
/// an initial/default entry, then optional section entries.
pub struct BootCatalog<'a> {
    /// Validation entry (first 32 bytes)
    pub validation: &'a ValidationEntry,

    /// Initial/default boot entry (next 32 bytes)
    pub initial: &'a BootEntry,
}

impl<'a> BootCatalog<'a> {
    /// Catalog entry size (32 bytes)
    pub const ENTRY_SIZE: usize = 32;

    /// Minimum catalog size (validation + initial entry)
    pub const MIN_SIZE: usize = Self::ENTRY_SIZE * 2;

    /// Parse boot catalog from sector data
    ///
    /// # Arguments
    /// * `data` - Raw sector data (at least 64 bytes)
    ///
    /// # Returns
    /// Parsed boot catalog with validation and initial entries
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < Self::MIN_SIZE {
            return Err(Iso9660Error::InvalidBootCatalog);
        }

        // Parse validation entry (first 32 bytes)
        let validation = unsafe { &*(data.as_ptr() as *const ValidationEntry) };

        if !validation.is_valid() {
            return Err(Iso9660Error::ChecksumFailed);
        }

        // Parse initial/default entry (next 32 bytes)
        let initial = unsafe { &*(data[32..].as_ptr() as *const BootEntry) };

        Ok(Self {
            validation,
            initial,
        })
    }

    /// Check if the catalog contains a bootable entry
    pub fn is_bootable(&self) -> bool {
        self.initial.is_bootable()
    }

    /// Get the platform ID from the validation entry
    pub fn platform_id(&self) -> u8 {
        self.validation.platform_id
    }
}
