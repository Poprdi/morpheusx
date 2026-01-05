//! Boot catalog parsing
//!
//! El Torito Boot Catalog structure and parsing.

use crate::error::{Iso9660Error, Result};

/// Boot Catalog
///
/// The boot catalog starts with a validation entry followed by
/// an initial/default entry, then optional section entries.
pub struct BootCatalog {
    /// Validation entry
    pub validation_entry: [u8; 32],
    
    /// Initial/default boot entry
    pub initial_entry: [u8; 32],
}

impl BootCatalog {
    /// Catalog entry size (32 bytes)
    pub const ENTRY_SIZE: usize = 32;
    
    /// Parse boot catalog from sector data
    pub fn parse(_data: &[u8]) -> Result<Self> {
        // TODO: Implementation
        // 1. Parse validation entry (header ID 0x01)
        // 2. Validate checksum
        // 3. Parse initial entry
        
        Err(Iso9660Error::InvalidBootCatalog)
    }
}
