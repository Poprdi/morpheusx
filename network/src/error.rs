//! Network error types

extern crate alloc;

use core::fmt;

pub type Result<T> = core::result::Result<T, NetworkError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkError {
    ProtocolNotAvailable,
    InitializationFailed,
    DnsResolutionFailed,
    ConnectionFailed,
    NotConnected,
    Timeout,
    HttpError(u16),
    InvalidUrl,
    InvalidResponse,
    FileError,
    OutOfMemory,
    Cancelled,
    TlsError,
    /// TLS/HTTPS is not supported - use HTTP URLs
    TlsNotSupported,
    /// Device-level error with description.
    DeviceError(alloc::string::String),
    /// Buffer too small for operation.
    BufferTooSmall,
    /// Response exceeded size limit.
    ResponseTooLarge,
    /// Too many redirects.
    TooManyRedirects,
    /// Send operation failed.
    SendFailed,
    /// Receive operation failed.
    ReceiveFailed,
    /// Receive error from device.
    ReceiveError,
    /// Unexpected end of stream.
    UnexpectedEof,
    /// Device not ready.
    DeviceNotReady,
    /// All buffers are in use.
    BufferExhausted,
    /// Packet too large to transmit.
    PacketTooLarge,
    Unknown,
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProtocolNotAvailable => write!(f, "Network protocol not available"),
            Self::InitializationFailed => write!(f, "Network initialization failed"),
            Self::DnsResolutionFailed => write!(f, "DNS resolution failed"),
            Self::ConnectionFailed => write!(f, "Connection failed"),
            Self::NotConnected => write!(f, "Not connected"),
            Self::Timeout => write!(f, "Request timed out"),
            Self::HttpError(code) => write!(f, "HTTP error: {}", code),
            Self::InvalidUrl => write!(f, "Invalid URL"),
            Self::InvalidResponse => write!(f, "Invalid response from server"),
            Self::FileError => write!(f, "File I/O error"),
            Self::OutOfMemory => write!(f, "Out of memory"),
            Self::Cancelled => write!(f, "Operation cancelled"),
            Self::TlsError => write!(f, "TLS/HTTPS error"),
            Self::TlsNotSupported => write!(f, "HTTPS not supported - use HTTP URL"),
            Self::DeviceError(msg) => write!(f, "Device error: {}", msg),
            Self::BufferTooSmall => write!(f, "Buffer too small"),
            Self::ResponseTooLarge => write!(f, "Response too large"),
            Self::TooManyRedirects => write!(f, "Too many redirects"),
            Self::SendFailed => write!(f, "Send failed"),
            Self::ReceiveFailed => write!(f, "Receive failed"),
            Self::ReceiveError => write!(f, "Device receive error"),
            Self::UnexpectedEof => write!(f, "Unexpected end of stream"),
            Self::DeviceNotReady => write!(f, "Device not ready"),
            Self::BufferExhausted => write!(f, "All buffers in use"),
            Self::PacketTooLarge => write!(f, "Packet too large"),
            Self::Unknown => write!(f, "Unknown error"),
        }
    }
}
