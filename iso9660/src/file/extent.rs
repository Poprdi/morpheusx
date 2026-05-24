//! Contiguous on-disc extent.

/// Contiguous region: starting LBA and byte length.
#[derive(Debug, Clone, Copy)]
pub struct Extent {
    /// Start LBA.
    pub lba: u32,
    /// Length in bytes.
    pub length: u32,
}

impl Extent {
    /// New extent at `lba` with `length` bytes.
    pub fn new(lba: u32, length: u32) -> Self {
        Self { lba, length }
    }

    pub fn sector_count(&self) -> u32 {
        self.length.div_ceil(2048)
    }

    /// One past the last sector.
    pub fn end_lba(&self) -> u32 {
        self.lba + self.sector_count()
    }
}
