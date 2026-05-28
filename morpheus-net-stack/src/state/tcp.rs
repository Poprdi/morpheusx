//! TCP connection state machine.
//!
//! Non-blocking TCP connection establishment and lifecycle management.
//!
//! # States
//! ```text
//! Closed → Connecting → Established → Closing → Closed
//!                 ↓           ↓           ↓
//!               Error       Error       Error
//! ```
//!
//! # Usage
//!
//! ```ignore
//! let mut tcp = TcpConnState::new();
//!
//! // Start connection (non-blocking)
//! tcp.initiate(socket_handle, remote_ip, remote_port, now_tsc);
//!
//! loop {
//!     iface.poll(...);
//!     
//!     // Get socket state from smoltcp
//!     let socket_state = get_tcp_state(socket_handle);
//!     
//!     match tcp.step(socket_state, now_tsc, timeout_ticks) {
//!         StepResult::Pending => continue,
//!         StepResult::Done => {
//!             let socket = tcp.socket().unwrap();
//!             // Use socket for send/recv
//!             break;
//!         }
//!         StepResult::Timeout => panic!("connect timeout"),
//!         StepResult::Failed => panic!("connect failed"),
//!     }
//! }
//! ```

use super::{StateError, StepResult, TscTimestamp};
use core::net::Ipv4Addr;

/// TCP socket state (simplified from smoltcp).
///
/// Used to communicate socket state from smoltcp to our state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TcpSocketState {
    /// Socket is closed
    Closed,
    /// Listening (server mode - not used here)
    Listen,
    /// SYN sent, waiting for SYN-ACK
    SynSent,
    /// SYN-ACK received, sending ACK
    SynReceived,
    /// Connection established
    Established,
    /// FIN sent, waiting for ACK
    FinWait1,
    /// FIN-ACK received
    FinWait2,
    /// Waiting for FIN from peer
    CloseWait,
    /// FIN sent after CloseWait
    Closing,
    /// FIN received in FinWait1
    LastAck,
    /// Waiting for timeout
    TimeWait,
}

impl TcpSocketState {
    /// Check if socket is connected and can send/receive.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Established | Self::CloseWait)
    }

    /// Check if connection attempt failed.
    pub fn is_failed(self) -> bool {
        // Closed after SynSent means connection refused
        self == Self::Closed
    }

    pub fn is_closing(self) -> bool {
        matches!(
            self,
            Self::FinWait1 | Self::FinWait2 | Self::Closing | Self::LastAck | Self::TimeWait
        )
    }
}

/// TCP-specific errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpError {
    /// Connection timed out
    ConnectTimeout,
    /// Connection refused by remote
    ConnectionRefused,
    /// Connection reset by remote
    ConnectionReset,
    /// Close timed out
    CloseTimeout,
    /// Socket error
    SocketError,
    /// Invalid state
    InvalidState,
}

impl From<TcpError> for StateError {
    fn from(e: TcpError) -> Self {
        match e {
            TcpError::ConnectTimeout | TcpError::CloseTimeout => StateError::Timeout,
            TcpError::ConnectionRefused => StateError::ConnectionRefused,
            TcpError::ConnectionReset => StateError::ConnectionReset,
            _ => StateError::ConnectionFailed,
        }
    }
}

/// Information about an established connection.
#[derive(Debug, Clone, Copy)]
pub struct TcpConnectionInfo {
    /// Local port
    pub local_port: u16,
    /// Remote IP address
    pub remote_ip: Ipv4Addr,
    /// Remote port
    pub remote_port: u16,
}

/// TCP connection state machine.
///
/// Manages non-blocking TCP connection establishment and closing.
/// Does NOT handle data transfer - that's done directly on the socket.
#[derive(Debug)]
pub(crate) enum TcpConnState {
    /// Socket not connected
    Closed,

    /// Connection initiated, waiting for establishment
    Connecting {
        /// Socket handle (opaque, passed to smoltcp)
        socket_handle: usize,
        /// Remote address
        remote_ip: Ipv4Addr,
        /// Remote port
        remote_port: u16,
        /// Local port (for reference)
        local_port: u16,
        /// When connect started
        start_tsc: TscTimestamp,
    },

    /// Connection established
    Established {
        /// Socket handle
        socket_handle: usize,
        /// Connection info
        info: TcpConnectionInfo,
    },

    /// Connection closing
    Closing {
        /// Socket handle
        socket_handle: usize,
        /// When close started
        start_tsc: TscTimestamp,
    },

    /// Error state
    Error {
        /// Error details
        error: TcpError,
    },
}

impl TcpConnState {
    /// Create new TCP state machine in closed state.
    pub fn new() -> Self {
        TcpConnState::Closed
    }

    /// Initiate connection.
    ///
    /// Called AFTER smoltcp's `socket.connect()` has been called.
    /// This just tracks the state - actual connect is done by smoltcp.
    pub fn initiate(
        &mut self,
        socket_handle: usize,
        remote_ip: Ipv4Addr,
        remote_port: u16,
        local_port: u16,
        now_tsc: u64,
    ) {
        *self = TcpConnState::Connecting {
            socket_handle,
            remote_ip,
            remote_port,
            local_port,
            start_tsc: TscTimestamp::new(now_tsc),
        };
    }

    /// Step the state machine.
    pub fn step(
        &mut self,
        socket_state: TcpSocketState,
        now_tsc: u64,
        timeout_ticks: u64,
    ) -> StepResult {
        match self {
            TcpConnState::Closed => {
                // Not started
                StepResult::Pending
            },

            TcpConnState::Connecting {
                socket_handle,
                remote_ip,
                remote_port,
                local_port,
                start_tsc,
            } => {
                // Check timeout first
                if start_tsc.is_expired(now_tsc, timeout_ticks) {
                    *self = TcpConnState::Error {
                        error: TcpError::ConnectTimeout,
                    };
                    return StepResult::Timeout;
                }

                // Check socket state
                if socket_state.is_active() {
                    // Connected!
                    let handle = *socket_handle;
                    let info = TcpConnectionInfo {
                        local_port: *local_port,
                        remote_ip: *remote_ip,
                        remote_port: *remote_port,
                    };
                    *self = TcpConnState::Established {
                        socket_handle: handle,
                        info,
                    };
                    return StepResult::Done;
                }

                if socket_state == TcpSocketState::Closed {
                    // Connection refused or reset
                    *self = TcpConnState::Error {
                        error: TcpError::ConnectionRefused,
                    };
                    return StepResult::Failed;
                }

                // Still connecting
                StepResult::Pending
            },

            TcpConnState::Established { .. } => {
                // Already connected
                StepResult::Done
            },

            TcpConnState::Closing {
                socket_handle,
                start_tsc,
            } => {
                // Check timeout
                if start_tsc.is_expired(now_tsc, timeout_ticks) {
                    *self = TcpConnState::Error {
                        error: TcpError::CloseTimeout,
                    };
                    return StepResult::Timeout;
                }

                // Check if fully closed
                if socket_state == TcpSocketState::Closed {
                    *self = TcpConnState::Closed;
                    return StepResult::Done;
                }

                // Still closing
                let _ = socket_handle;
                StepResult::Pending
            },

            TcpConnState::Error { error } => match error {
                TcpError::ConnectTimeout | TcpError::CloseTimeout => StepResult::Timeout,
                _ => StepResult::Failed,
            },
        }
    }

    /// Start graceful close.
    ///
    /// Called AFTER smoltcp's `socket.close()` has been called.
    pub fn close(&mut self, now_tsc: u64) {
        if let TcpConnState::Established { socket_handle, .. } = self {
            let handle = *socket_handle;
            *self = TcpConnState::Closing {
                socket_handle: handle,
                start_tsc: TscTimestamp::new(now_tsc),
            };
        }
    }

    pub fn abort(&mut self) {
        *self = TcpConnState::Closed;
    }

    /// Mark as failed with error.
    pub fn fail(&mut self, error: TcpError) {
        *self = TcpConnState::Error { error };
    }

    pub fn socket_handle(&self) -> Option<usize> {
        match self {
            TcpConnState::Connecting { socket_handle, .. } => Some(*socket_handle),
            TcpConnState::Established { socket_handle, .. } => Some(*socket_handle),
            TcpConnState::Closing { socket_handle, .. } => Some(*socket_handle),
            _ => None,
        }
    }

    pub fn connection_info(&self) -> Option<&TcpConnectionInfo> {
        match self {
            TcpConnState::Established { info, .. } => Some(info),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<TcpError> {
        match self {
            TcpConnState::Error { error } => Some(*error),
            _ => None,
        }
    }

    pub fn is_established(&self) -> bool {
        matches!(self, TcpConnState::Established { .. })
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, TcpConnState::Closed)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, TcpConnState::Error { .. })
    }

    /// Check if terminal (established, closed, or error).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TcpConnState::Established { .. } | TcpConnState::Closed | TcpConnState::Error { .. }
        )
    }
}

impl Default for TcpConnState {
    fn default() -> Self {
        Self::new()
    }
}
