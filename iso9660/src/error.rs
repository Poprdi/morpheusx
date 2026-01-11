//! Error types for ISO9660 operations

use core::fmt;

/// Result type for ISO9660 operations
pub type Result<T> = core::result::Result<T, Iso9660Error>;

/// Errors that can occur during ISO9660 operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Iso9660Error {
    /// I/O error reading from block device
    IoError,

    /// Invalid volume descriptor signature
    InvalidSignature,

    /// Unsupported ISO9660 version
    UnsupportedVersion,

    /// Corrupted directory record
    InvalidDirectoryRecord,

    /// File or directory not found
    NotFound,

    /// Path is too long
    PathTooLong,

    /// Invalid path format
    InvalidPath,

    /// File extent out of bounds
    ExtentOutOfBounds,

    /// Invalid file flags
    InvalidFlags,

    /// Invalid datetime format
    InvalidDatetime,

    /// Invalid string encoding
    InvalidString,

    /// Boot record not found
    NoBootRecord,

    /// Invalid boot catalog
    InvalidBootCatalog,

    /// No boot catalog found
    NoBootCatalog,

    /// Invalid boot entry
    InvalidBootEntry,

    /// Validation entry checksum failed
    ChecksumFailed,

    /// Unsupported boot platform
    UnsupportedPlatform,

    /// Rock Ridge extension error
    RockRidgeError,

    /// Joliet extension error
    JolietError,

    /// Read failed
    ReadFailed,

    /// Internal error (should not occur)
    InternalError,
}

impl fmt::Display for Iso9660Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IoError => write!(f, "I/O error reading block device"),
            Self::InvalidSignature => write!(f, "Invalid volume descriptor signature"),
            Self::UnsupportedVersion => write!(f, "Unsupported ISO9660 version"),
            Self::InvalidDirectoryRecord => write!(f, "Corrupted directory record"),
            Self::NotFound => write!(f, "File or directory not found"),
            Self::PathTooLong => write!(f, "Path exceeds maximum length"),
            Self::InvalidPath => write!(f, "Invalid path format"),
            Self::ExtentOutOfBounds => write!(f, "File extent out of bounds"),
            Self::InvalidFlags => write!(f, "Invalid file flags"),
            Self::InvalidDatetime => write!(f, "Invalid datetime format"),
            Self::InvalidString => write!(f, "Invalid string encoding"),
            Self::NoBootRecord => write!(f, "Boot record volume descriptor not found"),
            Self::InvalidBootCatalog => write!(f, "Invalid El Torito boot catalog"),
            Self::NoBootCatalog => write!(f, "No boot catalog found"),
            Self::InvalidBootEntry => write!(f, "Invalid boot entry"),
            Self::ChecksumFailed => write!(f, "Validation entry checksum failed"),
            Self::UnsupportedPlatform => write!(f, "Unsupported boot platform"),
            Self::RockRidgeError => write!(f, "Rock Ridge extension error"),
            Self::JolietError => write!(f, "Joliet extension error"),
            Self::ReadFailed => write!(f, "Read operation failed"),
            Self::InternalError => write!(f, "Internal error"),
        }
    }
}
