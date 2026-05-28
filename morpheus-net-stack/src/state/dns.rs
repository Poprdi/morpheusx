//! Non-blocking DNS resolver over smoltcp's DNS socket (Init -> Resolving ->
//! Resolved | Failed).

use super::{StateError, StepResult, TscTimestamp};
use core::net::Ipv4Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsError {
    Timeout,
    /// Server error, NXDOMAIN, etc.
    QueryFailed,
    NoIpv4Address,
    StartFailed,
    InvalidHostname,
}

crate::impl_from!(DnsError => StateError : DnsError(_));

#[derive(Debug)]
pub(crate) enum DnsResolveState {
    Init,
    Resolving {
        query_handle: usize,
        start_tsc: TscTimestamp,
    },
    Resolved {
        ip: Ipv4Addr,
    },
    Failed {
        error: DnsError,
    },
}

impl DnsResolveState {
    pub fn new() -> Self {
        DnsResolveState::Init
    }

    pub fn start(&mut self, query_handle: usize, now_tsc: u64) {
        *self = DnsResolveState::Resolving {
            query_handle,
            start_tsc: TscTimestamp::new(now_tsc),
        };
    }

    pub fn resolve(&mut self, ip: Ipv4Addr) {
        *self = DnsResolveState::Resolved { ip };
    }

    pub fn fail(&mut self, error: DnsError) {
        *self = DnsResolveState::Failed { error };
    }

    pub fn step(
        &mut self,
        dns_result: Result<Option<Ipv4Addr>, ()>,
        now_tsc: u64,
        timeout_ticks: u64,
    ) -> StepResult {
        match self {
            DnsResolveState::Init => StepResult::Pending,

            DnsResolveState::Resolving { start_tsc, .. } => {
                if start_tsc.is_expired(now_tsc, timeout_ticks) {
                    *self = DnsResolveState::Failed {
                        error: DnsError::Timeout,
                    };
                    return StepResult::Timeout;
                }

                match dns_result {
                    Ok(Some(ip)) => {
                        *self = DnsResolveState::Resolved { ip };
                        StepResult::Done
                    },
                    Ok(None) => StepResult::Pending,
                    Err(_) => {
                        *self = DnsResolveState::Failed {
                            error: DnsError::QueryFailed,
                        };
                        StepResult::Failed
                    },
                }
            },

            DnsResolveState::Resolved { .. } => StepResult::Done,
            DnsResolveState::Failed { error } => {
                if *error == DnsError::Timeout {
                    StepResult::Timeout
                } else {
                    StepResult::Failed
                }
            },
        }
    }

    pub fn query_handle(&self) -> Option<usize> {
        match self {
            DnsResolveState::Resolving { query_handle, .. } => Some(*query_handle),
            _ => None,
        }
    }

    pub fn ip(&self) -> Option<Ipv4Addr> {
        match self {
            DnsResolveState::Resolved { ip } => Some(*ip),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<DnsError> {
        match self {
            DnsResolveState::Failed { error } => Some(*error),
            _ => None,
        }
    }

    /// Resolved or failed.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            DnsResolveState::Resolved { .. } | DnsResolveState::Failed { .. }
        )
    }

    pub fn is_resolved(&self) -> bool {
        matches!(self, DnsResolveState::Resolved { .. })
    }
}

impl Default for DnsResolveState {
    fn default() -> Self {
        Self::new()
    }
}

/// Fallback hostname table for when DNS is unavailable or fails.
pub fn lookup_hardcoded(hostname: &str) -> Option<Ipv4Addr> {
    const KNOWN_HOSTS: &[(&str, [u8; 4])] = &[
        ("speedtest.tele2.net", [90, 130, 70, 73]),
        ("mirror.fcix.net", [204, 152, 191, 37]),
        ("ftp.acc.umu.se", [130, 239, 18, 159]),
        ("releases.ubuntu.com", [91, 189, 91, 38]),
        ("cdimage.ubuntu.com", [91, 189, 88, 142]),
        ("tails.net", [204, 13, 164, 63]),
        ("dl.amnesia.boum.org", [141, 138, 141, 92]),
        ("geo.mirror.pkgbuild.com", [143, 244, 34, 62]),
        ("mirror.rackspace.com", [162, 242, 93, 58]),
        ("deb.debian.org", [151, 101, 130, 132]),
        ("cdimage.debian.org", [194, 71, 11, 165]),
        ("download.fedoraproject.org", [38, 145, 60, 22]),
        ("cloudflare.com", [104, 16, 132, 229]),
    ];

    for (host, octets) in KNOWN_HOSTS {
        if hostname.eq_ignore_ascii_case(host) {
            return Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]));
        }
    }

    None
}

pub fn parse_ip(hostname: &str) -> Option<Ipv4Addr> {
    hostname.parse().ok()
}

/// IP literal or hardcoded lookup; None means a real DNS query is needed.
pub fn resolve_without_dns(hostname: &str) -> Option<Ipv4Addr> {
    parse_ip(hostname).or_else(|| lookup_hardcoded(hostname))
}
