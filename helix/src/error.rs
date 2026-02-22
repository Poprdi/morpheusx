//! Error types for HelixFS.

/// Unified error type for all Helix operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelixError {
    // ── I/O ──
    /// Block device read failed.
    IoReadFailed,
    /// Block device write failed.
    IoWriteFailed,
    /// Block device flush failed.
    IoFlushFailed,

    // ── Superblock ──
    /// Neither superblock has a valid magic / CRC.
    NoValidSuperblock,
    /// Superblock version mismatch.
    IncompatibleVersion,

    // ── Log ──
    /// The log is full and GC is required.
    LogFull,
    /// CRC mismatch on a log record.
    LogCrcMismatch,
    /// Log segment header is corrupt.
    LogSegmentCorrupt,

    // ── Index / B-tree ──
    /// Path not found in the namespace index.
    NotFound,
    /// Path already exists (e.g. mkdir on existing dir).
    AlreadyExists,
    /// B-tree depth exceeded implementation limit.
    IndexDepthExceeded,
    /// A B-tree node CRC is invalid.
    IndexCrcMismatch,

    // ── Filesystem structure ──
    /// Path exceeds maximum length (255 bytes).
    PathTooLong,
    /// Path contains invalid characters.
    PathInvalid,
    /// File exceeds maximum supported size.
    FileTooLarge,
    /// Out of free data blocks.
    NoSpace,
    /// Out of inode / index entry capacity.
    NoEntries,
    /// Target is a directory where a file was expected.
    IsADirectory,
    /// Extent data structure corrupt or invalid.
    ExtentCorrupt,

    // ── Bitmap ──
    /// Block bitmap is corrupt.
    BitmapCorrupt,

    // ── Transaction ──
    /// Transaction not in progress.
    NoActiveTransaction,
    /// Transaction conflict (concurrent modification).
    TxConflict,

    // ── Format ──
    /// Partition is too small.
    FormatTooSmall,
    /// Block size is not 4096.
    InvalidBlockSize,

    // ── VFS ──
    /// File descriptor is invalid / not open.
    InvalidFd,
    /// Maximum open files exceeded.
    TooManyOpenFiles,
    /// Operation not supported on this entry type.
    NotSupported,
    /// Attempt to write a read-only filesystem or descriptor.
    ReadOnly,
    /// Not a directory.
    NotADirectory,
    /// Directory is not empty.
    DirectoryNotEmpty,
    /// Mount table is full.
    MountTableFull,
    /// No mount handles this path.
    MountNotFound,
    /// Permission denied.
    PermissionDenied,
    /// Invalid seek offset / whence.
    InvalidOffset,
}
