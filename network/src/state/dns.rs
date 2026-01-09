//! DNS resolution state machine.
//!
//! Non-blocking DNS resolver that integrates with smoltcp's DNS socket.
//!
//! # States
//! Init → Resolving → Resolved | Failed
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5.3

use core::net::Ipv4Addr;
use super::{StepResult, StateError, TscTimestamp};

// ═══════════════════════════════════════════════════════════════════════════
// DNS ERROR
// ═══════════════════════════════════════════════════════════════════════════

/// DNS-specific errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsError {
    /// Query timed out
    Timeout,
    /// Query failed (server error, NXDOMAIN, etc.)
    QueryFailed,
    /// No IPv4 address in response
    NoIpv4Address,
    /// Failed to start query
    StartFailed,
    /// Invalid hostname
    InvalidHostname,
}

impl From<DnsError> for StateError {
    fn from(_: DnsError) -> Self {
        StateError::DnsError
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DNS RESOLVE STATE
// ═══════════════════════════════════════════════════════════════════════════

/// DNS resolution state machine.
///
/// Resolves a hostname to an IPv4 address using smoltcp's DNS socket.
#[derive(Debug)]
pub enum DnsResolveState {
    /// Initial state - not started
    Init,
    
    /// Waiting for DNS response
    Resolving {
        /// Query handle from smoltcp
        query_handle: usize,
        /// When query started (for timeout)
        start_tsc: TscTimestamp,
    },
    
    /// Resolution complete
    Resolved {
        /// Resolved IP address
        ip: Ipv4Addr,
    },
    
    /// Resolution failed
    Failed {
        /// Error details
        error: DnsError,
    },
}

impl DnsResolveState {
    /// Create new DNS resolver in init state.
    pub fn new() -> Self {
        DnsResolveState::Init
    }
    
    /// Start DNS resolution.
    ///
    /// # Arguments
    /// - `query_handle`: Handle returned from smoltcp's `start_query()`
    /// - `now_tsc`: Current TSC timestamp
    pub fn start(&mut self, query_handle: usize, now_tsc: u64) {
        *self = DnsResolveState::Resolving {
            query_handle,
            start_tsc: TscTimestamp::new(now_tsc),
        };
    }
    
    /// Mark as resolved with IP.
    pub fn resolve(&mut self, ip: Ipv4Addr) {
        *self = DnsResolveState::Resolved { ip };
    }
    
    /// Mark as failed.
    pub fn fail(&mut self, error: DnsError) {
        *self = DnsResolveState::Failed { error };
    }
    
    /// Step the state machine.
    ///
    /// # Arguments
    /// - `dns_result`: Result from smoltcp's `get_query_result()`:
    ///   - `Ok(Some(ip))`: Resolved
    ///   - `Ok(None)`: Still pending
    ///   - `Err(_)`: Failed
    /// - `now_tsc`: Current TSC value
    /// - `timeout_ticks`: DNS timeout in TSC ticks
    ///
    /// # Returns
    /// - `Pending`: Still resolving
    /// - `Done`: Resolved, call `ip()` to get result
    /// - `Timeout`: Query timed out
    /// - `Failed`: Query failed
    pub fn step(
        &mut self,
        dns_result: Result<Option<Ipv4Addr>, ()>,
        now_tsc: u64,
        timeout_ticks: u64,
    ) -> StepResult {
        match self {
            DnsResolveState::Init => {
                // Not started yet
                StepResult::Pending
            }
            
            DnsResolveState::Resolving { start_tsc, .. } => {
                // Check timeout first
                if start_tsc.is_expired(now_tsc, timeout_ticks) {
                    *self = DnsResolveState::Failed { error: DnsError::Timeout };
                    return StepResult::Timeout;
                }
                
                // Check DNS result
                match dns_result {
                    Ok(Some(ip)) => {
                        *self = DnsResolveState::Resolved { ip };
                        StepResult::Done
                    }
                    Ok(None) => {
                        // Still pending
                        StepResult::Pending
                    }
                    Err(_) => {
                        *self = DnsResolveState::Failed { error: DnsError::QueryFailed };
                        StepResult::Failed
                    }
                }
            }
            
            DnsResolveState::Resolved { .. } => StepResult::Done,
            DnsResolveState::Failed { error } => {
                if *error == DnsError::Timeout {
                    StepResult::Timeout
                } else {
                    StepResult::Failed
                }
            }
        }
    }
    
    /// Get query handle (if resolving).
    pub fn query_handle(&self) -> Option<usize> {
        match self {
            DnsResolveState::Resolving { query_handle, .. } => Some(*query_handle),
            _ => None,
        }
    }
    
    /// Get resolved IP address (if complete).
    pub fn ip(&self) -> Option<Ipv4Addr> {
        match self {
            DnsResolveState::Resolved { ip } => Some(*ip),
            _ => None,
        }
    }
    
    /// Get error (if failed).
    pub fn error(&self) -> Option<DnsError> {
        match self {
            DnsResolveState::Failed { error } => Some(*error),
            _ => None,
        }
    }
    
    /// Check if resolution is complete (success or failure).
    pub fn is_terminal(&self) -> bool {
        matches!(self, DnsResolveState::Resolved { .. } | DnsResolveState::Failed { .. })
    }
    
    /// Check if resolution succeeded.
    pub fn is_resolved(&self) -> bool {
        matches!(self, DnsResolveState::Resolved { .. })
    }
}

impl Default for DnsResolveState {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HARDCODED DNS FALLBACK
// ═══════════════════════════════════════════════════════════════════════════

/// Lookup hostname in hardcoded table.
///
/// Fallback for when DNS is unavailable or fails.
pub fn lookup_hardcoded(hostname: &str) -> Option<Ipv4Addr> {
    const KNOWN_HOSTS: &[(&str, [u8; 4])] = &[
        // Speed test servers
        ("speedtest.tele2.net", [90, 130, 70, 73]),
        // Mirror sites
        ("mirror.fcix.net", [204, 152, 191, 37]),
        ("ftp.acc.umu.se", [130, 239, 18, 159]),
        // Ubuntu mirrors
        ("releases.ubuntu.com", [91, 189, 91, 38]),
        ("cdimage.ubuntu.com", [91, 189, 88, 142]),
        // Tails
        ("tails.net", [204, 13, 164, 63]),
        ("dl.amnesia.boum.org", [141, 138, 141, 92]),
        // Arch Linux
        ("geo.mirror.pkgbuild.com", [143, 244, 34, 62]),
        ("mirror.rackspace.com", [162, 242, 93, 58]),
        // Debian
        ("deb.debian.org", [151, 101, 130, 132]),
        ("cdimage.debian.org", [194, 71, 11, 165]),
        // Fedora
        ("download.fedoraproject.org", [38, 145, 60, 22]),
        // Common CDNs
        ("cloudflare.com", [104, 16, 132, 229]),
    ];
    
    for (host, octets) in KNOWN_HOSTS {
        if hostname.eq_ignore_ascii_case(host) {
            return Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]));
        }
    }
    
    None
}

/// Parse hostname as IP address.
pub fn parse_ip(hostname: &str) -> Option<Ipv4Addr> {
    hostname.parse().ok()
}

/// Resolve hostname: try parse as IP, then hardcoded lookup.
///
/// Returns `None` if neither works (need actual DNS query).
pub fn resolve_without_dns(hostname: &str) -> Option<Ipv4Addr> {
    parse_ip(hostname).or_else(|| lookup_hardcoded(hostname))
}
