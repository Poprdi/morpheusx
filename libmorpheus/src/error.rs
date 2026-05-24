//! Structured error type wrapping raw kernel errnos.

use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    NotFound,
    ProcessNotFound,
    Io,
    BadFd,
    OutOfMemory,
    Fault,
    BrokenPipe,
    NotImplemented,
    InvalidInput,
    WouldBlock,
    TimedOut,
    AlreadyExists,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    NotConnected,
    UnexpectedEof,
    WriteZero,
    Unknown,
}

#[derive(Clone, Copy)]
pub struct Error {
    kind: ErrorKind,
    raw: u64,
}

impl Error {
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

    /// Synthetic error; raw = 0.
    #[inline]
    pub const fn new(kind: ErrorKind) -> Self {
        Self { kind, raw: 0 }
    }

    #[inline]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

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

impl From<u64> for Error {
    #[inline]
    fn from(raw: u64) -> Self {
        Self::from_raw(raw)
    }
}

pub type Result<T> = core::result::Result<T, Error>;

#[inline]
pub(crate) fn check(ret: u64) -> Result<u64> {
    if crate::is_error(ret) {
        Err(Error::from_raw(ret))
    } else {
        Ok(ret)
    }
}
