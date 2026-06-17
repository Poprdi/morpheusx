//! FAT32 engine error type.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fat32Error {
    IoRead,
    IoWrite,

    /// BPB boot signature / FAT32 markers absent or inconsistent.
    NotFat32,
    /// Cluster size, sectors-per-FAT, or sector size out of supported range.
    BadGeometry,
    /// Sector size is not what the backing device reports.
    InvalidBlockSize,

    NotFound,
    NotADirectory,
    IsADirectory,
    PathTooLong,
    PathInvalid,

    /// Cluster chain ran into a free/bad marker or looped past the file size.
    ChainCorrupt,
    InvalidOffset,

    /// Read-only engine; any mutator hits this.
    ReadOnly,
}

// `gpt_disk_io::BlockIo::Error` requires Display, so a BlockIo whose Error is
// Fat32Error (e.g. a RAM-backed device) needs this impl.
impl core::fmt::Display for Fat32Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self, f)
    }
}
