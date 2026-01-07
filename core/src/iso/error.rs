//! ISO storage error types
//!
//! Error enum for ISO chunk operations. Follows the same pattern as
//! `Fat32Error` in the fs module.

/// Errors that can occur during ISO chunk operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoError {
    /// Block I/O operation failed
    IoError,
    /// Invalid manifest format or magic number
    InvalidManifest,
    /// Manifest version not supported
    UnsupportedVersion,
    /// Chunk index out of bounds
    ChunkOutOfBounds,
    /// Not enough free partitions for chunks
    InsufficientPartitions,
    /// Partition too small for chunk
    PartitionTooSmall,
    /// ISO size exceeds maximum supported (16 chunks * 4GB)
    IsoTooLarge,
    /// Chunk partition not found by UUID
    ChunkNotFound,
    /// FAT32 filesystem error on chunk partition
    FilesystemError,
    /// Write position beyond current chunk
    WriteOverflow,
    /// Read position beyond ISO size
    ReadOverflow,
    /// Manifest already exists
    ManifestExists,
    /// No manifest found for ISO
    ManifestNotFound,
    /// SHA256 checksum mismatch
    ChecksumMismatch,
    /// Chunk data corrupted
    DataCorruption,
    /// Operation not supported
    NotSupported,
}

impl IsoError {
    /// Get a human-readable description of the error
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
