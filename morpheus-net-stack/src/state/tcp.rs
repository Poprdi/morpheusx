//! Non-blocking TCP connection state machine, tracking smoltcp socket state.
//!
//! Closed -> Connecting -> Established -> Closing -> Closed, with an Error
//! branch reachable from any active state.

use super::{StateError, StepResult, TscTimestamp};
use core::net::Ipv4Addr;

/// RFC 793 TCP states, mirrored from smoltcp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TcpSocketState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

impl TcpSocketState {
    /// Connected and able to send/receive.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Established | Self::CloseWait)
    }

    pub fn is_failed(self) -> bool {
        // Closed after SynSent means connection refused.
        self == Self::Closed
    }

    pub fn is_closing(self) -> bool {
        matches!(
            self,
            Self::FinWait1 | Self::FinWait2 | Self::Closing | Self::LastAck | Self::TimeWait
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpError {
    ConnectTimeout,
    ConnectionRefused,
    ConnectionReset,
    CloseTimeout,
    SocketError,
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

#[derive(Debug, Clone, Copy)]
pub struct TcpConnectionInfo {
    pub local_port: u16,
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,
}

/// Tracks connection setup and teardown; data transfer goes through the socket
/// directly, not this machine.
#[derive(Debug)]
pub(crate) enum TcpConnState {
    Closed,
    Connecting {
        socket_handle: usize,
        remote_ip: Ipv4Addr,
        remote_port: u16,
        local_port: u16,
        start_tsc: TscTimestamp,
    },
    Established {
        socket_handle: usize,
        info: TcpConnectionInfo,
    },
    Closing {
        socket_handle: usize,
        start_tsc: TscTimestamp,
    },
    Error {
        error: TcpError,
    },
}

impl TcpConnState {
    pub fn new() -> Self {
        TcpConnState::Closed
    }

    /// Call after smoltcp's `socket.connect()`; this only tracks state.
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

    pub fn step(
        &mut self,
        socket_state: TcpSocketState,
        now_tsc: u64,
        timeout_ticks: u64,
    ) -> StepResult {
        match self {
            TcpConnState::Closed => StepResult::Pending,

            TcpConnState::Connecting {
                socket_handle,
                remote_ip,
                remote_port,
                local_port,
                start_tsc,
            } => {
                if start_tsc.is_expired(now_tsc, timeout_ticks) {
                    *self = TcpConnState::Error {
                        error: TcpError::ConnectTimeout,
                    };
                    return StepResult::Timeout;
                }

                if socket_state.is_active() {
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
                    // Closed while connecting => refused or reset.
                    *self = TcpConnState::Error {
                        error: TcpError::ConnectionRefused,
                    };
                    return StepResult::Failed;
                }

                StepResult::Pending
            },

            TcpConnState::Established { .. } => StepResult::Done,

            TcpConnState::Closing {
                socket_handle,
                start_tsc,
            } => {
                if start_tsc.is_expired(now_tsc, timeout_ticks) {
                    *self = TcpConnState::Error {
                        error: TcpError::CloseTimeout,
                    };
                    return StepResult::Timeout;
                }

                if socket_state == TcpSocketState::Closed {
                    *self = TcpConnState::Closed;
                    return StepResult::Done;
                }

                let _ = socket_handle;
                StepResult::Pending
            },

            TcpConnState::Error { error } => match error {
                TcpError::ConnectTimeout | TcpError::CloseTimeout => StepResult::Timeout,
                _ => StepResult::Failed,
            },
        }
    }

    /// Call after smoltcp's `socket.close()`.
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

    /// Established, closed, or error.
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
