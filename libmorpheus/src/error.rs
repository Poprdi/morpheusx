//! Structured error type for MorpheusX syscalls and I/O.
//!
//! Every raw syscall returns `Result<T, u64>`.  This module provides a
//! richer [`Error`] type that maps kernel errno values to named variants,
//! and a [`Result<T>`] alias that uses it.
//!
//! # Converting from raw errors
//!
//! ```ignore
//! use libmorpheus::error::Error;
//! let e: Error = raw_errno.into();
//! match e.kind() {
//!     ErrorKind::NotFound => { /* handle */ }
//!     _ => { /* fallback */ }
//! }
//! ```

use core::fmt;

/// Error kind — maps 1:1 to kernel errno values, plus synthetic kinds
/// for higher-level operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// `ENOENT` — file or entity not found.
    NotFound,
    /// `ESRCH` — no such process.
    ProcessNotFound,
    /// `EIO` — I/O error.
    Io,
    /// `EBADF` — bad file descriptor.
    BadFd,
    /// `ENOMEM` — out of memory.
    OutOfMemory,
    /// `EFAULT` — bad address.
    Fault,
    /// `EPIPE` — broken pipe.
    BrokenPipe,
    /// `ENOSYS` — syscall not implemented.
    NotImplemented,
    /// `EINVAL` — invalid argument.
    InvalidInput,
    /// Non-blocking I/O returned 0 bytes.
    WouldBlock,
    /// Operation timed out.
    TimedOut,
    /// Entity already exists.
    AlreadyExists,
    /// Permission denied.
    PermissionDenied,
    /// Connection refused.
    ConnectionRefused,
    /// Connection reset by peer.
    ConnectionReset,
    /// Not connected.
    NotConnected,
    /// Unexpected end of file.
    UnexpectedEof,
    /// Write returned 0.
    WriteZero,
    /// Unrecognized error code.
    Unknown,
}

/// The error type for MorpheusX I/O and syscall operations.
///
/// Carries a [`ErrorKind`] for matching and the raw kernel u64 for
/// debugging.  16 bytes on the stack — cheap to copy.
#[derive(Clone, Copy)]
pub struct Error {
    kind: ErrorKind,
    raw: u64,
}

impl Error {
    /// Create an error from a raw kernel error code.
    #[inline]
    pub fn from_raw(raw: u64) -> Self {
        let kind = match raw {
            crate::ENOENT => ErrorKind::NotFound,
            crate::ESRCH => ErrorKind::ProcessNotFound,
            crate::EIO => ErrorKind::Io,
            crate::EBADF => ErrorKind::BadFd,
            crate::ENOMEM => ErrorKind::OutOfMemory,
            crate::EFAULT => ErrorKind::Fault,
            crate::EPIPE => ErrorKind::BrokenPipe,
            crate::ENOSYS => ErrorKind::NotImplemented,
            crate::EINVAL => ErrorKind::InvalidInput,
            _ => ErrorKind::Unknown,
        };
        Self { kind, raw }
    }

    /// Create a synthetic error from a kind (raw = 0).
    #[inline]
    pub const fn new(kind: ErrorKind) -> Self {
        Self { kind, raw: 0 }
    }

    /// The error kind for `match`-based dispatch.
    #[inline]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The raw kernel error code.  0 if this is a synthetic error.
    #[inline]
    pub const fn raw_code(&self) -> u64 {
        self.raw
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error({:?}, raw=0x{:x})", self.kind, self.raw)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ErrorKind::NotFound => f.write_str("not found"),
            ErrorKind::ProcessNotFound => f.write_str("no such process"),
            ErrorKind::Io => f.write_str("I/O error"),
            ErrorKind::BadFd => f.write_str("bad file descriptor"),
            ErrorKind::OutOfMemory => f.write_str("out of memory"),
            ErrorKind::Fault => f.write_str("bad address"),
            ErrorKind::BrokenPipe => f.write_str("broken pipe"),
            ErrorKind::NotImplemented => f.write_str("not implemented"),
            ErrorKind::InvalidInput => f.write_str("invalid argument"),
            ErrorKind::WouldBlock => f.write_str("would block"),
            ErrorKind::TimedOut => f.write_str("timed out"),
            ErrorKind::AlreadyExists => f.write_str("already exists"),
            ErrorKind::PermissionDenied => f.write_str("permission denied"),
            ErrorKind::ConnectionRefused => f.write_str("connection refused"),
            ErrorKind::ConnectionReset => f.write_str("connection reset"),
            ErrorKind::NotConnected => f.write_str("not connected"),
            ErrorKind::UnexpectedEof => f.write_str("unexpected end of file"),
            ErrorKind::WriteZero => f.write_str("write zero"),
            ErrorKind::Unknown => write!(f, "unknown error (0x{:x})", self.raw),
        }
    }
}

/// Convert a raw kernel u64 error to [`Error`].
impl From<u64> for Error {
    #[inline]
    fn from(raw: u64) -> Self {
        Self::from_raw(raw)
    }
}

/// Result type alias for MorpheusX operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Helper: convert a raw syscall return to `Result<u64>`.
///
/// Used internally by higher-level wrappers.
#[inline]
pub(crate) fn check(ret: u64) -> Result<u64> {
    if crate::is_error(ret) {
        Err(Error::from_raw(ret))
    } else {
        Ok(ret)
    }
}
