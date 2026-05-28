//! Shared context for the download state machine. Hardware arrives already
//! initialized from hwinit.

use smoltcp::iface::SocketHandle;
use smoltcp::wire::IpAddress;

use morpheus_block::device::UnifiedBlockDevice;

/// Network timeouts, all derived from the TSC frequency.
#[derive(Clone, Copy)]
pub struct Timeouts {
    tsc_freq: u64,
}

impl Timeouts {
    pub fn new(tsc_freq: u64) -> Self {
        Self { tsc_freq }
    }

    pub fn dhcp(&self) -> u64 {
        self.tsc_freq * 10
    }

    pub fn dns(&self) -> u64 {
        self.tsc_freq * 5
    }

    pub fn tcp_connect(&self) -> u64 {
        self.tsc_freq * 10
    }

    pub fn http_idle(&self) -> u64 {
        self.tsc_freq * 30
    }
}

#[derive(Clone)]
pub struct DownloadConfig<'a> {
    pub url: &'a str,
    pub write_to_disk: bool,
    pub target_start_sector: u64,
    /// Raw-write sector for the manifest.
    pub manifest_sector: u64,
    /// ESP start LBA for the FAT32 manifest.
    pub esp_start_lba: u64,
    pub partition_uuid: [u8; 16],
    pub iso_name: &'a str,
    /// 0 = unknown.
    pub expected_size: u64,
}

impl<'a> DownloadConfig<'a> {
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

    /// Download + disk write + manifest.
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

pub struct Context<'a> {
    pub timeouts: Timeouts,
    pub tsc_freq: u64,
    pub config: DownloadConfig<'a>,
    pub dhcp_handle: Option<SocketHandle>,
    pub dns_handle: Option<SocketHandle>,
    pub tcp_handle: Option<SocketHandle>,
    pub blk_device: Option<UnifiedBlockDevice>,
    pub resolved_ip: Option<IpAddress>,
    pub resolved_port: u16,
    pub url_path: &'a str,
    pub url_host: &'a str,
    pub content_length: Option<u64>,
    pub bytes_downloaded: u64,
    pub bytes_written: u64,
    pub current_write_sector: u64,
    pub dns_servers: [Option<IpAddress>; 3],
    /// May differ from config after GPT prep.
    pub actual_start_sector: u64,
}

impl<'a> Context<'a> {
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

    pub fn with_block_device(mut self, device: UnifiedBlockDevice) -> Self {
        self.blk_device = Some(device);
        self
    }

    pub fn url(&self) -> &str {
        self.config.url
    }

    pub fn should_write_to_disk(&self) -> bool {
        self.config.write_to_disk && self.blk_device.is_some()
    }
}

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
