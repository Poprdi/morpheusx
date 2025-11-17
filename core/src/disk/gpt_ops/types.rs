// Common types for GPT operations

#[derive(Copy, Clone, Debug)]
pub enum GptError {
    IoError,
    InvalidHeader,
    NoSpace,
    PartitionNotFound,
    OverlappingPartitions,
    InvalidSize,
    AlignmentError,
}

/// Represents a free space region on disk
#[derive(Copy, Clone, Debug)]
pub struct FreeRegion {
    pub start_lba: u64,
    pub end_lba: u64,
}

impl FreeRegion {
    pub fn size_lba(&self) -> u64 {
        self.end_lba - self.start_lba + 1
    }

    pub fn size_mb(&self) -> u64 {
        (self.size_lba() * 512) / (1024 * 1024)
    }
}
