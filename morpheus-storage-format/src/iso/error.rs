//! ISO chunk-storage errors.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoError {
    IoError,
    InvalidManifest,
    UnsupportedVersion,
    ChunkOutOfBounds,
    InsufficientPartitions,
    PartitionTooSmall,
    /// ISO exceeds 16 chunks * 4 GB.
    IsoTooLarge,
    ChunkNotFound,
    FilesystemError,
    WriteOverflow,
    ReadOverflow,
    ManifestExists,
    ManifestNotFound,
    ChecksumMismatch,
    DataCorruption,
    NotSupported,
}

impl IsoError {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::IoError => "Block I/O operation failed",
            Self::InvalidManifest => "Invalid manifest format",
            Self::UnsupportedVersion => "Unsupported manifest version",
            Self::ChunkOutOfBounds => "Chunk index out of bounds",
            Self::InsufficientPartitions => "Not enough partitions for chunks",
            Self::PartitionTooSmall => "Partition too small for chunk",
            Self::IsoTooLarge => "ISO exceeds maximum size (64GB)",
            Self::ChunkNotFound => "Chunk partition not found",
            Self::FilesystemError => "FAT32 filesystem error",
            Self::WriteOverflow => "Write beyond chunk boundary",
            Self::ReadOverflow => "Read beyond ISO size",
            Self::ManifestExists => "Manifest already exists",
            Self::ManifestNotFound => "Manifest not found",
            Self::ChecksumMismatch => "SHA256 checksum mismatch",
            Self::DataCorruption => "Chunk data corrupted",
            Self::NotSupported => "Operation not supported",
        }
    }
}
