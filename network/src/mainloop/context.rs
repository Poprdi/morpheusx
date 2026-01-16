//! Shared context for the download state machine.
//!
//! Self-contained — no handoff dependencies.
//! Network receives already-initialized hardware from hwinit.

use smoltcp::iface::SocketHandle;
use smoltcp::wire::IpAddress;

use crate::device::UnifiedBlockDevice;

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

/// Full download configuration.
#[derive(Clone)]
pub struct DownloadConfig<'a> {
    /// URL to download
    pub url: &'a str,
    /// Write to disk?
    pub write_to_disk: bool,
    /// Target start sector for ISO
    pub target_start_sector: u64,
    /// Sector for manifest (raw write)
    pub manifest_sector: u64,
    /// ESP start LBA (for FAT32 manifest)
    pub esp_start_lba: u64,
    /// Partition UUID
    pub partition_uuid: [u8; 16],
    /// ISO name for manifest
    pub iso_name: &'a str,
    /// Expected ISO size (0 = unknown)
    pub expected_size: u64,
}

impl<'a> DownloadConfig<'a> {
    /// Simple config for download-only (no disk write).
    pub fn download_only(url: &'a str) -> Self {
        Self {
            url,
            write_to_disk: false,
            target_start_sector: 0,
            manifest_sector: 0,
            esp_start_lba: 0,
            partition_uuid: [0u8; 16],
            iso_name: "",
            expected_size: 0,
        }
    }

    /// Full config for download + disk write + manifest.
    pub fn full(
        url: &'a str,
        target_start_sector: u64,
        manifest_sector: u64,
        esp_start_lba: u64,
        partition_uuid: [u8; 16],
        iso_name: &'a str,
    ) -> Self {
        Self {
            url,
            write_to_disk: true,
            target_start_sector,
            manifest_sector,
            esp_start_lba,
            partition_uuid,
            iso_name,
            expected_size: 0,
        }
    }
}

/// Shared context passed between states.
pub struct Context<'a> {
    /// Timeout configuration
    pub timeouts: Timeouts,
    /// TSC frequency
    pub tsc_freq: u64,
    /// Full configuration
    pub config: DownloadConfig<'a>,
    /// DHCP socket handle
    pub dhcp_handle: Option<SocketHandle>,
    /// DNS socket handle
    pub dns_handle: Option<SocketHandle>,
    /// TCP socket handle
    pub tcp_handle: Option<SocketHandle>,
    /// Block device for disk writes
    pub blk_device: Option<UnifiedBlockDevice>,
    /// Resolved IP address (from DNS)
    pub resolved_ip: Option<IpAddress>,
    /// Resolved port
    pub resolved_port: u16,
    /// Path portion of URL
    pub url_path: &'a str,
    /// Host portion of URL
    pub url_host: &'a str,
    /// Content-Length from HTTP response
    pub content_length: Option<u64>,
    /// Total bytes downloaded
    pub bytes_downloaded: u64,
    /// Total bytes written to disk
    pub bytes_written: u64,
    /// Current write sector
    pub current_write_sector: u64,
    /// DNS servers from DHCP
    pub dns_servers: [Option<IpAddress>; 3],
    /// Actual start sector (after GPT prep, may differ from config)
    pub actual_start_sector: u64,
}

impl<'a> Context<'a> {
    /// Create new context.
    pub fn new(config: DownloadConfig<'a>, tsc_freq: u64) -> Self {
        let start_sector = config.target_start_sector;
        Self {
            timeouts: Timeouts::new(tsc_freq),
            tsc_freq,
            config,
            dhcp_handle: None,
            dns_handle: None,
            tcp_handle: None,
            blk_device: None,
            resolved_ip: None,
            resolved_port: 80,
            url_path: "",
            url_host: "",
            content_length: None,
            bytes_downloaded: 0,
            bytes_written: 0,
            current_write_sector: start_sector,
            dns_servers: [None; 3],
            actual_start_sector: start_sector,
        }
    }

    /// Set block device for disk writes.
    pub fn with_block_device(mut self, device: UnifiedBlockDevice) -> Self {
        self.blk_device = Some(device);
        self
    }

    /// Get URL from config.
    pub fn url(&self) -> &str {
        self.config.url
    }

    /// Check if disk write is enabled.
    pub fn should_write_to_disk(&self) -> bool {
        self.config.write_to_disk && self.blk_device.is_some()
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
