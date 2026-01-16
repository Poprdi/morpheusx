//! Top-level network stack entry point.
//!
//! This is THE entry point for the network stack. The bootloader calls this
//! ONCE after hwinit has set up the platform. We do everything:
//! - PCI scan for NICs
//! - Driver initialization (brutal reset)
//! - DHCP, DNS, HTTP
//! - Download to disk
//!
//! # Architecture
//!
//! ```text
//! Bootloader
//!     │
//!     ├── ExitBootServices
//!     │
//!     ├── hwinit::platform_init_selfcontained()
//!     │   └── Memory, GDT, IDT, PIC, heap, TSC, DMA, bus mastering
//!     │
//!     └── network::run_download()  ← THIS MODULE
//!         ├── scan_for_nic()       (our job)
//!         ├── create driver        (our job)
//!         ├── brutal reset         (our job)
//!         └── download_with_config (our job)
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // After hwinit returns:
//! let result = network::run_download(RunConfig {
//!     dma_region: platform.dma_region,
//!     tsc_freq: platform.tsc_freq,
//!     url: "http://example.com/image.iso",
//!     iso_name: "image.iso",
//!     esp_start_lba: 2048,
//! });
//! ```

use crate::dma::DmaRegion;
use crate::boot::probe::{scan_for_nic, DetectedNic, ProbeError};
use crate::driver::virtio::{VirtioConfig, VirtioNetDriver};
use crate::driver::intel::{E1000eConfig, E1000eDriver};
use crate::mainloop::{download_with_config, DownloadConfig, DownloadResult};
use crate::mainloop::serial::{print, println, print_hex};

// ═══════════════════════════════════════════════════════════════════════════
// CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// Configuration for network download.
///
/// Pass this to `run_download()` with everything needed.
/// Note: url and iso_name must be 'static because the download
/// orchestrator stores them in static context.
pub struct RunConfig<'a> {
    /// DMA region from hwinit (identity-mapped, safe for device DMA)
    pub dma_region: &'a DmaRegion,
    /// TSC frequency from hwinit (for timing)
    pub tsc_freq: u64,
    /// URL to download (must be 'static)
    pub url: &'static str,
    /// ISO name for manifest (must be 'static)
    pub iso_name: &'static str,
    /// ESP start LBA (0 = don't write to disk)
    pub esp_start_lba: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// RESULT
// ═══════════════════════════════════════════════════════════════════════════

/// Result of network download operation.
#[derive(Debug, Clone, Copy)]
pub enum RunResult {
    /// Download completed successfully
    Success {
        /// Bytes downloaded
        bytes: u64,
    },
    /// No network device found
    NoDevice,
    /// Driver initialization failed
    DriverInitFailed,
    /// Download failed
    DownloadFailed,
}

// ═══════════════════════════════════════════════════════════════════════════
// ENTRY POINT
// ═══════════════════════════════════════════════════════════════════════════

/// Run a complete network download.
///
/// This is the top-level entry point. We handle everything:
/// - PCI scan for NICs
/// - Driver creation and brutal reset
/// - DHCP, DNS, HTTP download
///
/// # Preconditions
/// - hwinit has completed (GDT, IDT, PIC, heap, DMA, bus mastering)
/// - Platform is sane
///
/// # Safety
/// - Must be called after hwinit
/// - DMA region must be valid
pub unsafe fn run_download(config: RunConfig<'_>) -> RunResult {
    println("[NET] Network stack starting");
    println("[NET] Scanning for NIC...");

    // Step 1: Find a NIC
    let nic = match scan_for_nic() {
        Some(n) => n,
        None => {
            println("[NET] ERROR: No network device found");
            return RunResult::NoDevice;
        }
    };

    // Step 2: Create driver and run download
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

/// Run download with VirtIO driver.
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

/// Run download with Intel e1000e driver.
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

/// Run download with any driver that implements NetworkDriver.
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
            print_hex(bytes_written as u64);
            println(" bytes");
            RunResult::Success { bytes: bytes_written as u64 }
        }
        DownloadResult::Failed { reason } => {
            print("[NET] Download failed: ");
            println(reason);
            RunResult::DownloadFailed
        }
    }
}
