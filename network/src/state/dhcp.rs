//! DHCP client state machine.
//!
//! Wraps smoltcp's DHCP client with non-blocking state tracking.
//! Does NOT implement DHCP protocol itself—relies on smoltcp.
//!
//! # States
//! Init → Discovering → Bound | Failed
//!
//! # Usage
//!
//! ```ignore
//! let mut dhcp = DhcpState::new();
//! dhcp.start(now_tsc);
//!
//! loop {
//!     iface.poll(...);  // smoltcp handles DHCP internally
//!     
//!     // Check if smoltcp has assigned an IP
//!     let has_ip = iface.ipv4_addr().is_some();
//!     
//!     match dhcp.step(has_ip, now_tsc, timeout_ticks) {
//!         StepResult::Pending => continue,
//!         StepResult::Done => {
//!             let config = dhcp.config().unwrap();
//!             break;
//!         }
//!         StepResult::Timeout => panic!("DHCP timeout"),
//!         StepResult::Failed => panic!("DHCP failed"),
//!     }
//! }
//! ```
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5.3

use core::net::Ipv4Addr;
use super::{StepResult, StateError, TscTimestamp};

// ═══════════════════════════════════════════════════════════════════════════
// DHCP ERROR
// ═══════════════════════════════════════════════════════════════════════════

/// DHCP-specific errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpError {
    /// Discovery timed out
    Timeout,
    /// No interface available
    NoInterface,
    /// Lease expired
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

// ═══════════════════════════════════════════════════════════════════════════
// DHCP CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// DHCP configuration obtained from server.
#[derive(Debug, Clone, Copy)]
pub struct DhcpConfig {
    /// Assigned IP address
    pub ip: Ipv4Addr,
    /// Subnet mask as prefix length (e.g., 24 for /24)
    pub prefix_len: u8,
    /// Default gateway
    pub gateway: Option<Ipv4Addr>,
    /// Primary DNS server
    pub dns: Option<Ipv4Addr>,
}

// ═══════════════════════════════════════════════════════════════════════════
// DHCP STATE MACHINE
// ═══════════════════════════════════════════════════════════════════════════

/// DHCP client state machine.
///
/// Tracks DHCP discovery progress and wraps smoltcp's DHCP socket.
#[derive(Debug)]
pub enum DhcpState {
    /// Initial state - not started
    Init,
    
    /// DHCP discovery in progress
    Discovering {
        /// When discovery started
        start_tsc: TscTimestamp,
    },
    
    /// IP address obtained
    Bound {
        /// DHCP configuration
        config: DhcpConfig,
        /// When bound
        bound_tsc: TscTimestamp,
    },
    
    /// DHCP failed
    Failed {
        /// Error details
        error: DhcpError,
    },
}

impl DhcpState {
    /// Create new DHCP state machine in init state.
    pub fn new() -> Self {
        DhcpState::Init
    }
    
    /// Start DHCP discovery.
    ///
    /// # Arguments
    /// - `now_tsc`: Current TSC timestamp
    pub fn start(&mut self, now_tsc: u64) {
        *self = DhcpState::Discovering {
            start_tsc: TscTimestamp::new(now_tsc),
        };
    }
    
    /// Step the state machine.
    ///
    /// # Arguments
    /// - `dhcp_config`: Current DHCP config from smoltcp (None if not yet configured)
    /// - `now_tsc`: Current TSC value
    /// - `timeout_ticks`: DHCP timeout in TSC ticks
    ///
    /// # Returns
    /// - `Pending`: Still discovering
    /// - `Done`: Bound, call `config()` to get configuration
    /// - `Timeout`: Discovery timed out
    /// - `Failed`: Discovery failed
    pub fn step(
        &mut self,
        dhcp_config: Option<DhcpConfig>,
        now_tsc: u64,
        timeout_ticks: u64,
    ) -> StepResult {
        match self {
            DhcpState::Init => {
                // Not started yet
                StepResult::Pending
            }
            
            DhcpState::Discovering { start_tsc } => {
                // Check timeout first
                if start_tsc.is_expired(now_tsc, timeout_ticks) {
                    *self = DhcpState::Failed { error: DhcpError::Timeout };
                    return StepResult::Timeout;
                }
                
                // Check if smoltcp has configured an IP
                if let Some(config) = dhcp_config {
                    *self = DhcpState::Bound {
                        config,
                        bound_tsc: TscTimestamp::new(now_tsc),
                    };
                    return StepResult::Done;
                }
                
                // Still discovering
                StepResult::Pending
            }
            
            DhcpState::Bound { .. } => StepResult::Done,
            
            DhcpState::Failed { error } => {
                if *error == DhcpError::Timeout {
                    StepResult::Timeout
                } else {
                    StepResult::Failed
                }
            }
        }
    }
    
    /// Get DHCP configuration (if bound).
    pub fn config(&self) -> Option<&DhcpConfig> {
        match self {
            DhcpState::Bound { config, .. } => Some(config),
            _ => None,
        }
    }
    
    /// Get assigned IP address (if bound).
    pub fn ip(&self) -> Option<Ipv4Addr> {
        self.config().map(|c| c.ip)
    }
    
    /// Get gateway (if bound and available).
    pub fn gateway(&self) -> Option<Ipv4Addr> {
        self.config().and_then(|c| c.gateway)
    }
    
    /// Get DNS server (if bound and available).
    pub fn dns(&self) -> Option<Ipv4Addr> {
        self.config().and_then(|c| c.dns)
    }
    
    /// Get error (if failed).
    pub fn error(&self) -> Option<DhcpError> {
        match self {
            DhcpState::Failed { error } => Some(*error),
            _ => None,
        }
    }
    
    /// Check if DHCP is complete (bound or failed).
    pub fn is_terminal(&self) -> bool {
        matches!(self, DhcpState::Bound { .. } | DhcpState::Failed { .. })
    }
    
    /// Check if successfully bound.
    pub fn is_bound(&self) -> bool {
        matches!(self, DhcpState::Bound { .. })
    }
    
    /// Check if still discovering.
    pub fn is_discovering(&self) -> bool {
        matches!(self, DhcpState::Discovering { .. })
    }
}

impl Default for DhcpState {
    fn default() -> Self {
        Self::new()
    }
}
