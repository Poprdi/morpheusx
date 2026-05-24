//! HelixFS error type.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelixError {
    IoReadFailed,
    IoWriteFailed,
    IoFlushFailed,

    /// Neither superblock has a valid magic / CRC.
    NoValidSuperblock,
    IncompatibleVersion,

    /// Log is full; GC required.
    LogFull,
    LogCrcMismatch,
    LogSegmentCorrupt,

    NotFound,
    AlreadyExists,
    IndexDepthExceeded,
    IndexCrcMismatch,

    /// Path exceeds 255 bytes.
    PathTooLong,
    PathInvalid,
    FileTooLarge,
    NoSpace,
    NoEntries,
    IsADirectory,
    ExtentCorrupt,

    BitmapCorrupt,

    NoActiveTransaction,
    TxConflict,

    FormatTooSmall,
    /// Block size is not 4096.
    InvalidBlockSize,

    InvalidFd,
    TooManyOpenFiles,
    NotSupported,
    ReadOnly,
    NotADirectory,
    DirectoryNotEmpty,
    MountTableFull,
    MountNotFound,
    PermissionDenied,
    InvalidOffset,
}
