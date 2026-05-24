//! El Torito boot catalog parsing.

use super::entry::BootEntry;
use super::validation::ValidationEntry;
use crate::error::{Iso9660Error, Result};

/// Boot catalog view: validation entry followed by the initial/default entry.
/// Section entries past byte 64 are not modeled.
pub struct BootCatalog<'a> {
    /// Validation entry (first 32 bytes).
    pub validation: &'a ValidationEntry,
    /// Initial/default boot entry (bytes 32..64).
    pub initial: &'a BootEntry,
}

impl<'a> BootCatalog<'a> {
    /// Each catalog entry is 32 bytes.
    pub const ENTRY_SIZE: usize = 32;

    /// Validation + initial entry.
    pub const MIN_SIZE: usize = Self::ENTRY_SIZE * 2;

    /// Parse the first two entries of a boot catalog from sector data.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < Self::MIN_SIZE {
            return Err(Iso9660Error::InvalidBootCatalog);
        }

        // SAFETY: length checked above; ValidationEntry is repr(C, packed) of 32 bytes
        // and borrows from `data` for the returned lifetime.
        let validation = unsafe { &*(data.as_ptr() as *const ValidationEntry) };

        if !validation.is_valid() {
            return Err(Iso9660Error::ChecksumFailed);
        }

        // SAFETY: same as above, offset 32 within the validated slice.
        let initial = unsafe { &*(data[32..].as_ptr() as *const BootEntry) };

        Ok(Self {
            validation,
            initial,
        })
    }

    /// Whether the initial entry is marked bootable.
    pub fn is_bootable(&self) -> bool {
        self.initial.is_bootable()
    }

    /// Platform ID from the validation entry.
    pub fn platform_id(&self) -> u8 {
        self.validation.platform_id
    }
}
