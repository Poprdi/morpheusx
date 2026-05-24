//! Platform-neutral PE/COFF parsing.

pub mod compile_time;
pub mod embedded_reloc;
pub mod embedded_reloc_data;
pub mod header;
pub mod reloc;
pub mod section;

use core::fmt;

#[derive(Debug, Clone, Copy)]
pub enum PeError {
    InvalidSignature,
    InvalidOffset,
    UnsupportedFormat,
    MissingSection,
    CorruptedData,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeArch {
    X64,
    ARM64,
    ARM,
}

pub type PeResult<T> = Result<T, PeError>;
