//! PE/COFF file format parsing
//!
//! Platform-neutral implementation of PE file structure parsing.
//! Works identically on x86_64, ARM64, and ARM32.

pub mod compile_time;
pub mod embedded_reloc;
pub mod embedded_reloc_data;
pub mod header;
pub mod reloc;
pub mod section;

use core::fmt;

/// Errors during PE parsing
#[derive(Debug, Clone, Copy)]
pub enum PeError {
    InvalidSignature,  // Not a valid PE file
    InvalidOffset,     // Offset out of bounds
    UnsupportedFormat, // PE32 vs PE32+ mismatch
    MissingSection,    // Required section not found
    CorruptedData,     // Data integrity check failed
}

impl fmt::Display for PeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PeError::InvalidSignature => write!(f, "Invalid PE signature"),
            PeError::InvalidOffset => write!(f, "Offset out of bounds"),
            PeError::UnsupportedFormat => write!(f, "Unsupported PE format"),
            PeError::MissingSection => write!(f, "Missing required section"),
            PeError::CorruptedData => write!(f, "Corrupted PE data"),
        }
    }
}

/// PE file architecture
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeArch {
    X64,   // x86_64 (PE32+)
    ARM64, // aarch64 (PE32+)
    ARM,   // armv7 (PE32)
}

/// Result type for PE operations
pub type PeResult<T> = Result<T, PeError>;
