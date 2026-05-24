//! DEPRECATED: thin wrapper over `mainloop` download orchestrator (kept for bringup).
//! TODO: rework once the real network path stabilizes.

use crate::boot::probe::{scan_for_nic, DetectedNic, ProbeError};
use crate::dma::DmaRegion;
use crate::driver::intel::{E1000eConfig, E1000eDriver};
use crate::driver::virtio::{VirtioConfig, VirtioNetDriver};
use crate::mainloop::serial::{print, print_hex, println};
use crate::mainloop::{download_with_config, DownloadConfig, DownloadResult};

/// url/iso_name are `'static` because the orchestrator stashes them in statics.
pub struct RunConfig<'a> {
    pub dma_region: &'a DmaRegion,
    pub tsc_freq: u64,
    pub url: &'static str,
    pub iso_name: &'static str,
    /// 0 disables disk write.
    pub esp_start_lba: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum RunResult {
    Success { bytes: u64 },
    NoDevice,
    DriverInitFailed,
    DownloadFailed,
}

/// PCI-scan, init driver, run download. Caller must have completed hwinit.
///
/// # Safety
/// DMA region must be valid and hwinit must have finished.
pub unsafe fn run_download(config: RunConfig<'_>) -> RunResult {
    println("[NET] Network stack starting");
    println("[NET] Scanning for NIC...");

    let nic = match scan_for_nic() {
        Some(n) => n,
        None => {
            println("[NET] ERROR: No network device found");
            return RunResult::NoDevice;
        }
    };

    match nic {
        DetectedNic::VirtIO { mmio_base, .. } => {
            print("[NET] Found VirtIO-net @ ");
            print_hex(mmio_base);
            println("");
            run_with_virtio(config, mmio_base)
        }
        DetectedNic::Intel(info) => {
            print("[NET] Found Intel e1000e @ ");
            print_hex(info.mmio_base);
            println("");
            run_with_intel(config, info.mmio_base)
        }
    }
}

unsafe fn run_with_virtio(config: RunConfig<'_>, mmio_base: u64) -> RunResult {
    let virtio_cfg = VirtioConfig {
        dma_cpu_base: config.dma_region.cpu_base(),
        dma_bus_base: config.dma_region.bus_base(),
        dma_size: config.dma_region.size(),
        queue_size: VirtioConfig::DEFAULT_QUEUE_SIZE,
        buffer_size: VirtioConfig::DEFAULT_BUFFER_SIZE,
    };

    let mut driver = match VirtioNetDriver::new(mmio_base, virtio_cfg) {
        Ok(d) => d,
        Err(_) => {
            println("[NET] VirtIO driver init failed");
            return RunResult::DriverInitFailed;
        }
    };

    println("[NET] VirtIO driver initialized");
    run_download_with_driver(&mut driver, config)
}

unsafe fn run_with_intel(config: RunConfig<'_>, mmio_base: u64) -> RunResult {
    let intel_cfg = E1000eConfig {
        dma_cpu_base: config.dma_region.cpu_base(),
        dma_bus_base: config.dma_region.bus_base(),
        rx_queue_size: 32,
        tx_queue_size: 32,
        buffer_size: 2048,
        tsc_freq: config.tsc_freq,
    };

    let mut driver = match E1000eDriver::new(mmio_base, intel_cfg) {
        Ok(d) => d,
        Err(_) => {
            println("[NET] Intel driver init failed");
            return RunResult::DriverInitFailed;
        }
    };

    println("[NET] Intel e1000e driver initialized");
    run_download_with_driver(&mut driver, config)
}

fn run_download_with_driver<D: crate::driver::traits::NetworkDriver>(
    driver: &mut D,
    config: RunConfig<'_>,
) -> RunResult {
    let download_config = DownloadConfig {
        url: config.url,
        write_to_disk: config.esp_start_lba > 0,
        target_start_sector: 0,
        manifest_sector: 0,
        esp_start_lba: config.esp_start_lba,
        partition_uuid: [0u8; 16],
        iso_name: config.iso_name,
        expected_size: 0,
    };

    let result = download_with_config(driver, download_config, None, config.tsc_freq);

    match result {
        DownloadResult::Success { bytes_written, .. } => {
            print("[NET] Download complete: ");
            print_hex(bytes_written);
            println(" bytes");
            RunResult::Success {
                bytes: bytes_written,
            }
        }
        DownloadResult::Failed { reason } => {
            print("[NET] Download failed: ");
            println(reason);
            RunResult::DownloadFailed
        }
    }
}
