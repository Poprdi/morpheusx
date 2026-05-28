//! Non-blocking state tracking over smoltcp's DHCP client (Init -> Discovering
//! -> Bound | Failed). The protocol itself is handled by smoltcp.

use super::{StateError, StepResult, TscTimestamp};
use core::net::Ipv4Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpError {
    Timeout,
    NoInterface,
    LeaseExpired,
}

impl From<DhcpError> for StateError {
    fn from(e: DhcpError) -> Self {
        match e {
            DhcpError::Timeout => StateError::Timeout,
            _ => StateError::InterfaceError,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DhcpConfig {
    pub ip: Ipv4Addr,
    /// Subnet mask as prefix length (e.g. 24 for /24).
    pub prefix_len: u8,
    pub gateway: Option<Ipv4Addr>,
    pub dns: Option<Ipv4Addr>,
}

#[derive(Debug)]
pub(crate) enum DhcpState {
    Init,
    Discovering {
        start_tsc: TscTimestamp,
    },
    Bound {
        config: DhcpConfig,
        bound_tsc: TscTimestamp,
    },
    Failed {
        error: DhcpError,
    },
}

impl DhcpState {
    pub fn new() -> Self {
        DhcpState::Init
    }

    pub fn start(&mut self, now_tsc: u64) {
        *self = DhcpState::Discovering {
            start_tsc: TscTimestamp::new(now_tsc),
        };
    }

    pub fn step(
        &mut self,
        dhcp_config: Option<DhcpConfig>,
        now_tsc: u64,
        timeout_ticks: u64,
    ) -> StepResult {
        match self {
            DhcpState::Init => StepResult::Pending,

            DhcpState::Discovering { start_tsc } => {
                if start_tsc.is_expired(now_tsc, timeout_ticks) {
                    *self = DhcpState::Failed {
                        error: DhcpError::Timeout,
                    };
                    return StepResult::Timeout;
                }

                if let Some(config) = dhcp_config {
                    *self = DhcpState::Bound {
                        config,
                        bound_tsc: TscTimestamp::new(now_tsc),
                    };
                    return StepResult::Done;
                }

                StepResult::Pending
            },

            DhcpState::Bound { .. } => StepResult::Done,

            DhcpState::Failed { error } => {
                if *error == DhcpError::Timeout {
                    StepResult::Timeout
                } else {
                    StepResult::Failed
                }
            },
        }
    }

    pub fn config(&self) -> Option<&DhcpConfig> {
        match self {
            DhcpState::Bound { config, .. } => Some(config),
            _ => None,
        }
    }

    pub fn ip(&self) -> Option<Ipv4Addr> {
        self.config().map(|c| c.ip)
    }

    pub fn gateway(&self) -> Option<Ipv4Addr> {
        self.config().and_then(|c| c.gateway)
    }

    pub fn dns(&self) -> Option<Ipv4Addr> {
        self.config().and_then(|c| c.dns)
    }

    pub fn error(&self) -> Option<DhcpError> {
        match self {
            DhcpState::Failed { error } => Some(*error),
            _ => None,
        }
    }

    /// Bound or failed.
    pub fn is_terminal(&self) -> bool {
        matches!(self, DhcpState::Bound { .. } | DhcpState::Failed { .. })
    }

    pub fn is_bound(&self) -> bool {
        matches!(self, DhcpState::Bound { .. })
    }

    pub fn is_discovering(&self) -> bool {
        matches!(self, DhcpState::Discovering { .. })
    }
}

impl Default for DhcpState {
    fn default() -> Self {
        Self::new()
    }
}
