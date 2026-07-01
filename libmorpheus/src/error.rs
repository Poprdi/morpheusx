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
    Interrupted,
    TimedOut,
    AlreadyExists,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    NotConnected,
    AddrInUse,
    AddrNotAvailable,
    NetworkUnreachable,
    HostUnreachable,
    UnexpectedEof,
    WriteZero,
    Unknown,
}

/// Maps a positive Linux-numeric errno to an [`ErrorKind`], mirroring
/// `std::sys::pal::unix::decode_error_kind` so the future std PAL reuses it.
#[inline]
pub fn decode_error_kind(errno: i32) -> ErrorKind {
    match errno {
        1 | 13 => ErrorKind::PermissionDenied,
        2 => ErrorKind::NotFound,
        3 => ErrorKind::ProcessNotFound,
        4 => ErrorKind::Interrupted,
        5 => ErrorKind::Io,
        9 => ErrorKind::BadFd,
        11 => ErrorKind::WouldBlock,
        12 => ErrorKind::OutOfMemory,
        14 => ErrorKind::Fault,
        17 => ErrorKind::AlreadyExists,
        22 => ErrorKind::InvalidInput,
        32 => ErrorKind::BrokenPipe,
        38 => ErrorKind::NotImplemented,
        98 => ErrorKind::AddrInUse,
        99 => ErrorKind::AddrNotAvailable,
        101 => ErrorKind::NetworkUnreachable,
        103 => ErrorKind::ConnectionAborted,
        104 => ErrorKind::ConnectionReset,
        107 => ErrorKind::NotConnected,
        110 => ErrorKind::TimedOut,
        111 => ErrorKind::ConnectionRefused,
        113 => ErrorKind::HostUnreachable,
        _ => ErrorKind::Unknown,
    }
}

#[derive(Clone, Copy)]
pub struct Error {
    kind: ErrorKind,
    raw: u64,
}

impl Error {
    #[inline]
    pub fn from_raw(raw: u64) -> Self {
        Self {
            kind: decode_error_kind(morpheus_foundation::errno::errno_value(raw)),
            raw,
        }
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
            ErrorKind::Interrupted => f.write_str("interrupted"),
            ErrorKind::TimedOut => f.write_str("timed out"),
            ErrorKind::AlreadyExists => f.write_str("already exists"),
            ErrorKind::PermissionDenied => f.write_str("permission denied"),
            ErrorKind::ConnectionRefused => f.write_str("connection refused"),
            ErrorKind::ConnectionReset => f.write_str("connection reset"),
            ErrorKind::ConnectionAborted => f.write_str("connection aborted"),
            ErrorKind::NotConnected => f.write_str("not connected"),
            ErrorKind::AddrInUse => f.write_str("address in use"),
            ErrorKind::AddrNotAvailable => f.write_str("address not available"),
            ErrorKind::NetworkUnreachable => f.write_str("network unreachable"),
            ErrorKind::HostUnreachable => f.write_str("host unreachable"),
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
