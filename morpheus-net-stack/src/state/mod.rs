//! Non-blocking network state machines. Each exposes `step()`, which returns
//! immediately; timeouts are observed via elapsed-time checks, never spun on.

pub mod dhcp;
pub mod disk_writer;
pub mod dns;
pub mod download;
pub mod http;
pub mod tcp;

use crate::time::TimeoutConfig;
use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    Pending,
    /// Success; call `output()` for the result.
    Done,
    /// Timed out; call `error()` for details.
    Timeout,
    /// Failed; call `error()` for details.
    Failed,
}

impl StepResult {
    #[inline]
    pub fn is_pending(self) -> bool {
        self == Self::Pending
    }

    #[inline]
    pub fn is_done(self) -> bool {
        self == Self::Done
    }

    /// Done, timeout, or failed.
    #[inline]
    pub fn is_terminal(self) -> bool {
        !self.is_pending()
    }

    /// Timeout or failed.
    #[inline]
    pub fn is_error(self) -> bool {
        matches!(self, Self::Timeout | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    Timeout,
    InterfaceError,
    SocketError,
    DnsError,
    ConnectionFailed,
    ConnectionRefused,
    ConnectionReset,
    HttpError,
    InvalidResponse,
    HttpStatus(u16),
    /// `step()` called before `start()`.
    NotStarted,
    AlreadyComplete,
    BufferTooSmall,
    Internal,
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "operation timed out"),
            Self::InterfaceError => write!(f, "network interface error"),
            Self::SocketError => write!(f, "socket error"),
            Self::DnsError => write!(f, "DNS resolution failed"),
            Self::ConnectionFailed => write!(f, "TCP connection failed"),
            Self::ConnectionRefused => write!(f, "connection refused"),
            Self::ConnectionReset => write!(f, "connection reset"),
            Self::HttpError => write!(f, "HTTP protocol error"),
            Self::InvalidResponse => write!(f, "invalid response"),
            Self::HttpStatus(code) => write!(f, "HTTP status {}", code),
            Self::NotStarted => write!(f, "operation not started"),
            Self::AlreadyComplete => write!(f, "operation already complete"),
            Self::BufferTooSmall => write!(f, "buffer too small"),
            Self::Internal => write!(f, "internal error"),
        }
    }
}

/// TSC timestamp for timeout math.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TscTimestamp(u64);

impl TscTimestamp {
    #[inline]
    pub const fn new(tsc: u64) -> Self {
        Self(tsc)
    }

    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Wrapping subtraction tolerates TSC overflow.
    #[inline]
    pub fn elapsed(self, now: u64) -> u64 {
        now.wrapping_sub(self.0)
    }

    #[inline]
    pub fn is_expired(self, now: u64, timeout_ticks: u64) -> bool {
        self.elapsed(now) > timeout_ticks
    }
}

impl From<u64> for TscTimestamp {
    fn from(tsc: u64) -> Self {
        Self(tsc)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StepContext {
    pub now_tsc: u64,
    pub timeouts: TimeoutConfig,
}

impl StepContext {
    pub fn new(now_tsc: u64, tsc_freq: u64) -> Self {
        Self {
            now_tsc,
            timeouts: TimeoutConfig::new(tsc_freq),
        }
    }

    #[inline]
    pub fn now(&self) -> TscTimestamp {
        TscTimestamp(self.now_tsc)
    }

    #[inline]
    pub fn is_expired(&self, start: TscTimestamp, timeout_ticks: u64) -> bool {
        start.is_expired(self.now_tsc, timeout_ticks)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    pub bytes_done: u64,
    /// 0 if unknown.
    pub bytes_total: u64,
    pub start_tsc: u64,
    pub last_update_tsc: u64,
}

impl Progress {
    pub fn new(now_tsc: u64) -> Self {
        Self {
            bytes_done: 0,
            bytes_total: 0,
            start_tsc: now_tsc,
            last_update_tsc: now_tsc,
        }
    }

    pub fn update(&mut self, bytes_done: u64, now_tsc: u64) {
        self.bytes_done = bytes_done;
        self.last_update_tsc = now_tsc;
    }

    pub fn set_total(&mut self, total: u64) {
        self.bytes_total = total;
    }

    /// 0-100, or 0 if total unknown.
    pub fn percentage(&self) -> u8 {
        if self.bytes_total == 0 {
            0
        } else {
            ((self.bytes_done * 100) / self.bytes_total) as u8
        }
    }

    pub fn bytes_per_second(&self, now_tsc: u64, tsc_freq: u64) -> u64 {
        let elapsed = now_tsc.wrapping_sub(self.start_tsc);
        if elapsed == 0 {
            return 0;
        }
        // bytes_done * tsc_freq / elapsed, dividing first if it would overflow.
        if self.bytes_done > u64::MAX / tsc_freq {
            (self.bytes_done / elapsed) * tsc_freq
        } else {
            (self.bytes_done * tsc_freq) / elapsed
        }
    }
}

pub(crate) use dhcp::DhcpState;
pub(crate) use dns::DnsResolveState;
pub(crate) use download::IsoDownloadState;
pub(crate) use http::HttpDownloadState;
pub(crate) use tcp::TcpConnState;
