//! ISO 9660 error types.

use core::fmt;

/// Result alias for ISO 9660 operations.
pub type Result<T> = core::result::Result<T, Iso9660Error>;

/// Errors surfaced by the ISO 9660 reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Iso9660Error {
    /// Block device I/O failure.
    IoError,
    /// Volume descriptor signature mismatch.
    InvalidSignature,
    /// Volume descriptor version not supported.
    UnsupportedVersion,
    /// Directory record failed structural checks.
    InvalidDirectoryRecord,
    /// File or directory not found.
    NotFound,
    /// Path exceeds maximum length.
    PathTooLong,
    /// Malformed path.
    InvalidPath,
    /// Extent reference outside the volume.
    ExtentOutOfBounds,
    /// Reserved bits set in directory record flags.
    InvalidFlags,
    /// Invalid 7.x.x datetime encoding.
    InvalidDatetime,
    /// String could not be decoded.
    InvalidString,
    /// No Boot Record volume descriptor present.
    NoBootRecord,
    /// El Torito boot catalog structurally invalid.
    InvalidBootCatalog,
    /// Boot catalog not found.
    NoBootCatalog,
    /// El Torito boot entry malformed.
    InvalidBootEntry,
    /// El Torito validation entry checksum mismatch.
    ChecksumFailed,
    /// Unrecognized El Torito platform ID.
    UnsupportedPlatform,
    /// Rock Ridge SUSP/RRIP parse error.
    RockRidgeError,
    /// Joliet decoding error.
    JolietError,
    /// Read operation failed.
    ReadFailed,
    /// Reached a path the code believed unreachable.
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
