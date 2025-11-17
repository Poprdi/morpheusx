//! Network error types

use core::fmt;

pub type Result<T> = core::result::Result<T, NetworkError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkError {
    ProtocolNotAvailable,
    InitializationFailed,
    DnsResolutionFailed,
    ConnectionFailed,
    Timeout,
    HttpError(u16),
    InvalidUrl,
    InvalidResponse,
    FileError,
    OutOfMemory,
    Cancelled,
    TlsError,
    Unknown,
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProtocolNotAvailable => write!(f, "Network protocol not available"),
            Self::InitializationFailed => write!(f, "Network initialization failed"),
            Self::DnsResolutionFailed => write!(f, "DNS resolution failed"),
            Self::ConnectionFailed => write!(f, "Connection failed"),
            Self::Timeout => write!(f, "Request timed out"),
            Self::HttpError(code) => write!(f, "HTTP error: {}", code),
            Self::InvalidUrl => write!(f, "Invalid URL"),
            Self::InvalidResponse => write!(f, "Invalid response from server"),
            Self::FileError => write!(f, "File I/O error"),
            Self::OutOfMemory => write!(f, "Out of memory"),
            Self::Cancelled => write!(f, "Operation cancelled"),
            Self::TlsError => write!(f, "TLS/HTTPS error"),
            Self::Unknown => write!(f, "Unknown error"),
        }
    }
}
