//! State machine module.
//!
//! Non-blocking state machines for all network operations.
//! Each state machine has a `step()` method that returns immediately.
//!
//! # Design Principles
//!
//! 1. **No blocking**: `step()` returns immediately, never waits
//! 2. **Timeout as observation**: Check elapsed time, don't spin
//! 3. **State transitions**: Move to next state when condition met
//! 4. **Composable**: State machines can contain other state machines
//!
//! # Usage Pattern
//!
//! ```ignore
//! loop {
//!     // Poll network stack (exactly once per iteration)
//!     iface.poll(now, device, sockets);
//!     
//!     // Step state machine (returns immediately)
//!     match state_machine.step(now_tsc, &timeouts) {
//!         StepResult::Pending => continue,
//!         StepResult::Done => break,
//!         StepResult::Timeout => handle_timeout(),
//!         StepResult::Failed => handle_error(),
//!     }
//! }
//! ```
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5

pub mod dhcp;
pub mod disk_writer;
pub mod dns;
pub mod download;
pub mod http;
pub mod tcp;

use crate::time::TimeoutConfig;
use core::fmt;

// ═══════════════════════════════════════════════════════════════════════════
// STEP RESULT
// ═══════════════════════════════════════════════════════════════════════════

/// Result of a state machine step.
///
/// Each call to `step()` returns one of these values to indicate
/// the current state of the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// Operation in progress, call `step()` again next iteration.
    Pending,

    /// Operation completed successfully.
    /// Call `output()` to get the result.
    Done,

    /// Operation timed out.
    /// Call `error()` for details.
    Timeout,

    /// Operation failed.
    /// Call `error()` for details.
    Failed,
}

impl StepResult {
    /// Check if operation is still in progress.
    #[inline]
    pub fn is_pending(self) -> bool {
        self == Self::Pending
    }

    /// Check if operation completed successfully.
    #[inline]
    pub fn is_done(self) -> bool {
        self == Self::Done
    }

    /// Check if operation terminated (done, timeout, or failed).
    #[inline]
    pub fn is_terminal(self) -> bool {
        !self.is_pending()
    }

    /// Check if operation ended in error (timeout or failed).
    #[inline]
    pub fn is_error(self) -> bool {
        matches!(self, Self::Timeout | Self::Failed)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STATE MACHINE ERROR
// ═══════════════════════════════════════════════════════════════════════════

/// Common error type for state machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    /// Operation timed out
    Timeout,
    /// Network interface error
    InterfaceError,
    /// Socket error
    SocketError,
    /// DNS resolution failed
    DnsError,
    /// TCP connection failed
    ConnectionFailed,
    /// TCP connection refused
    ConnectionRefused,
    /// TCP connection reset
    ConnectionReset,
    /// HTTP protocol error
    HttpError,
    /// Invalid response
    InvalidResponse,
    /// HTTP status error (status code)
    HttpStatus(u16),
    /// Not started (step called before start)
    NotStarted,
    /// Already completed
    AlreadyComplete,
    /// Buffer too small
    BufferTooSmall,
    /// Internal error
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

// ═══════════════════════════════════════════════════════════════════════════
// TIMESTAMP WRAPPER
// ═══════════════════════════════════════════════════════════════════════════

/// TSC timestamp wrapper for timeout calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TscTimestamp(u64);

impl TscTimestamp {
    /// Create from raw TSC value.
    #[inline]
    pub const fn new(tsc: u64) -> Self {
        Self(tsc)
    }

    /// Get raw TSC value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Calculate elapsed ticks since this timestamp.
    /// Uses wrapping subtraction to handle overflow.
    #[inline]
    pub fn elapsed(self, now: u64) -> u64 {
        now.wrapping_sub(self.0)
    }

    /// Check if timeout has elapsed.
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

// ═══════════════════════════════════════════════════════════════════════════
// STATE MACHINE CONTEXT
// ═══════════════════════════════════════════════════════════════════════════

/// Context passed to state machine step functions.
///
/// Contains timing information and configuration needed by all state machines.
#[derive(Debug, Clone, Copy)]
pub struct StepContext {
    /// Current TSC value
    pub now_tsc: u64,
    /// Timeout configuration
    pub timeouts: TimeoutConfig,
}

impl StepContext {
    /// Create new context.
    pub fn new(now_tsc: u64, tsc_freq: u64) -> Self {
        Self {
            now_tsc,
            timeouts: TimeoutConfig::new(tsc_freq),
        }
    }

    /// Create timestamp for current time.
    #[inline]
    pub fn now(&self) -> TscTimestamp {
        TscTimestamp(self.now_tsc)
    }

    /// Check if timeout has elapsed since start.
    #[inline]
    pub fn is_expired(&self, start: TscTimestamp, timeout_ticks: u64) -> bool {
        start.is_expired(self.now_tsc, timeout_ticks)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PROGRESS TRACKING
// ═══════════════════════════════════════════════════════════════════════════

/// Progress information for long-running operations.
#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    /// Bytes completed
    pub bytes_done: u64,
    /// Total bytes (0 if unknown)
    pub bytes_total: u64,
    /// Start timestamp
    pub start_tsc: u64,
    /// Last update timestamp
    pub last_update_tsc: u64,
}

impl Progress {
    /// Create new progress tracker.
    pub fn new(now_tsc: u64) -> Self {
        Self {
            bytes_done: 0,
            bytes_total: 0,
            start_tsc: now_tsc,
            last_update_tsc: now_tsc,
        }
    }

    /// Update progress.
    pub fn update(&mut self, bytes_done: u64, now_tsc: u64) {
        self.bytes_done = bytes_done;
        self.last_update_tsc = now_tsc;
    }

    /// Set total bytes (when known).
    pub fn set_total(&mut self, total: u64) {
        self.bytes_total = total;
    }

    /// Get percentage complete (0-100), or 0 if total unknown.
    pub fn percentage(&self) -> u8 {
        if self.bytes_total == 0 {
            0
        } else {
            ((self.bytes_done * 100) / self.bytes_total) as u8
        }
    }

    /// Calculate bytes per second (approximate).
    pub fn bytes_per_second(&self, now_tsc: u64, tsc_freq: u64) -> u64 {
        let elapsed = now_tsc.wrapping_sub(self.start_tsc);
        if elapsed == 0 {
            return 0;
        }
        // bytes_done * tsc_freq / elapsed
        // Avoid overflow by dividing first if needed
        if self.bytes_done > u64::MAX / tsc_freq {
            (self.bytes_done / elapsed) * tsc_freq
        } else {
            (self.bytes_done * tsc_freq) / elapsed
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use dhcp::DhcpState;
pub use dns::DnsResolveState;
pub use download::IsoDownloadState;
pub use http::HttpDownloadState;
pub use tcp::TcpConnState;
