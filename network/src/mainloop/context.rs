//! Shared context for the download state machine.
//!
//! Self-contained — no handoff dependencies.
//! Network receives already-initialized hardware from hwinit.

use smoltcp::iface::SocketHandle;
use smoltcp::wire::IpAddress;

/// Timeout configuration for network operations.
#[derive(Clone, Copy)]
pub struct Timeouts {
    tsc_freq: u64,
}

impl Timeouts {
    pub fn new(tsc_freq: u64) -> Self {
        Self { tsc_freq }
    }

    /// DHCP timeout (10 seconds).
    pub fn dhcp(&self) -> u64 {
        self.tsc_freq * 10
    }

    /// DNS timeout (5 seconds).
    pub fn dns(&self) -> u64 {
        self.tsc_freq * 5
    }

    /// TCP connect timeout (10 seconds).
    pub fn tcp_connect(&self) -> u64 {
        self.tsc_freq * 10
    }

    /// HTTP idle timeout (30 seconds).
    pub fn http_idle(&self) -> u64 {
        self.tsc_freq * 30
    }
}

/// Configuration for the download operation.
#[derive(Clone)]
pub struct DownloadConfig<'a> {
    /// URL to download
    pub url: &'a str,
}

impl<'a> DownloadConfig<'a> {
    pub fn new(url: &'a str) -> Self {
        Self { url }
    }
}

/// Shared context passed between states.
pub struct Context<'a> {
    /// Timeout configuration
    pub timeouts: Timeouts,
    /// TSC frequency
    pub tsc_freq: u64,
    /// Download URL
    pub url: &'a str,
    /// DHCP socket handle
    pub dhcp_handle: Option<SocketHandle>,
    /// DNS socket handle
    pub dns_handle: Option<SocketHandle>,
    /// TCP socket handle
    pub tcp_handle: Option<SocketHandle>,
    /// Resolved IP address (from DNS)
    pub resolved_ip: Option<IpAddress>,
    /// Resolved port
    pub resolved_port: u16,
    /// Path portion of URL
    pub url_path: &'a str,
    /// Host portion of URL
    pub url_host: &'a str,
    /// Content-Length from HTTP response (if known)
    pub content_length: Option<u64>,
    /// Total bytes downloaded so far
    pub bytes_downloaded: u64,
}

impl<'a> Context<'a> {
    /// Create new context.
    pub fn new(url: &'a str, tsc_freq: u64) -> Self {
        Self {
            timeouts: Timeouts::new(tsc_freq),
            tsc_freq,
            url,
            dhcp_handle: None,
            dns_handle: None,
            tcp_handle: None,
            resolved_ip: None,
            resolved_port: 80,
            url_path: "",
            url_host: "",
            content_length: None,
            bytes_downloaded: 0,
        }
    }
}

/// Read TSC (Time Stamp Counter).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn get_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nostack, nomem, preserves_flags)
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn get_tsc() -> u64 {
    0
}
