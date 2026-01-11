//! File extent management
//!
//! Extents represent contiguous data regions on disk.

/// File extent (contiguous data region)
#[derive(Debug, Clone, Copy)]
pub struct Extent {
    /// Starting LBA
    pub lba: u32,

    /// Length in bytes
    pub length: u32,
}

impl Extent {
    /// Create new extent
    pub fn new(lba: u32, length: u32) -> Self {
        Self { lba, length }
    }

    /// Number of sectors (2048 bytes each)
    pub fn sector_count(&self) -> u32 {
        self.length.div_ceil(2048)
    }

    /// End LBA (exclusive)
    pub fn end_lba(&self) -> u32 {
        self.lba + self.sector_count()
    }
}
