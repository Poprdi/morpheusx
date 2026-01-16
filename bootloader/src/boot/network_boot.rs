//! Network boot integration for post-ExitBootServices ISO download.
//!
//! # NEW ARCHITECTURE (hwinit split)
//!
//! ```text
//! UEFI Phase (pre-EBS):
//!   1. Allocate DMA region
//!   2. Calibrate TSC  
//!   3. Allocate stack
//!   4. Query GOP framebuffer
//!   5. Exit boot services
//!
//! Bare-metal Phase (post-EBS):
//!   6. hwinit::platform_init() - PCI scan, bus mastering, BAR decode
//!   7. Create driver from hwinit output
//!   8. network::download_with_config() - state machine
//! ```
//!
//! The old BootHandoff structure is DEPRECATED. Now hwinit handles
//! device discovery and the network driver is constructed directly.

#![allow(dead_code)]
#![allow(unused_imports)]

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::ToString;

use morpheus_hwinit::{
    platform_init, InitError, NetDeviceType, BlkDeviceType,
    PlatformConfig, PlatformInit, PreparedNetDevice, PreparedBlkDevice,
};
use morpheus_network::boot::handoff::BootHandoff;
use morpheus_network::driver::traits::NetworkDriver;
use morpheus_network::driver::virtio::{VirtioConfig, VirtioNetDriver};
use morpheus_network::driver::intel::{E1000eConfig, E1000eDriver};
use morpheus_network::mainloop::{download_with_config, DownloadConfig, DownloadResult};
use morpheus_network::device::UnifiedBlockDevice;

/// Network boot result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunResult {
    /// Download completed successfully
    Success,
    /// Download failed
    Failed,
    /// Hardware init failed
    HwInitFailed,
    /// No network device found
    NoDevice,
}

/// Configuration for bare-metal download (new architecture).
pub struct BaremetalConfig {
    /// URL to download
    pub iso_url: &'static str,
    /// ISO filename for manifest
    pub iso_name: &'static str,
    /// ESP start LBA for disk writes
    pub esp_start_lba: u64,
    /// Target start sector for ISO data
    pub target_start_sector: u64,
}

impl Default for BaremetalConfig {
    fn default() -> Self {
        Self {
            iso_url: "http://10.0.2.2:8000/test.iso",
            iso_name: "test.iso",
            esp_start_lba: 0,
            target_start_sector: 0,
        }
    }
}

/// NEW: Bare-metal entry point using hwinit architecture.
///
/// Call this AFTER ExitBootServices with pre-allocated resources.
///
/// # Safety
/// - Must be after ExitBootServices
/// - DMA region must be identity-mapped
/// - All resources must survive EBS
pub unsafe fn enter_baremetal_download(
    dma_base: *mut u8,
    dma_bus: u64,
    dma_size: usize,
    tsc_freq: u64,
    config: BaremetalConfig,
) -> RunResult {
    use morpheus_hwinit::serial::{puts, put_hex64, newline};
    
    puts("[BOOT] entering bare-metal download\n");
    puts("[BOOT] dma_base=");
    put_hex64(dma_base as u64);
    puts(" tsc_freq=");
    put_hex64(tsc_freq);
    newline();

    // Step 1: Run hwinit to scan PCI and enable bus mastering
    let platform_config = PlatformConfig {
        dma_base,
        dma_bus,
        dma_size,
        tsc_freq,
    };

    let platform = match platform_init(platform_config) {
        Ok(p) => p,
        Err(e) => {
            puts("[BOOT] hwinit failed: ");
            match e {
                InitError::InvalidDmaRegion => puts("invalid DMA region"),
                InitError::NoDevicesFound => puts("no devices found"),
                InitError::BarDecodeFailed => puts("BAR decode failed"),
                InitError::TscCalibrationFailed => puts("TSC calibration failed"),
                InitError::NoFreeMemory => puts("no free memory"),
            }
            newline();
            return RunResult::HwInitFailed;
        }
    };

    // Step 2: Find a network device
    let net_dev = match platform.net_devices.iter().find_map(|d| *d) {
        Some(d) => d,
        None => {
            puts("[BOOT] ERROR: no network device found\n");
            return RunResult::NoDevice;
        }
    };

    puts("[BOOT] using net device: ");
    match net_dev.device_type {
        NetDeviceType::VirtIO => puts("VirtIO"),
        NetDeviceType::IntelE1000e => puts("Intel e1000e"),
    }
    puts(" @ ");
    put_hex64(net_dev.mmio_base);
    newline();

    // Step 4: Create download config
    let download_config = DownloadConfig {
        url: config.iso_url,
        write_to_disk: config.esp_start_lba > 0,
        target_start_sector: config.target_start_sector,
        manifest_sector: 0,
        esp_start_lba: config.esp_start_lba,
        partition_uuid: [0u8; 16],
        iso_name: config.iso_name,
        expected_size: 0,
    };

    // Step 5: Create driver (this does brutal reset) and run download
    let result = match net_dev.device_type {
        NetDeviceType::VirtIO => {
            let virtio_cfg = VirtioConfig {
                dma_cpu_base: dma_base,
                dma_bus_base: dma_bus,
                dma_size,
                queue_size: VirtioConfig::DEFAULT_QUEUE_SIZE,
                buffer_size: VirtioConfig::DEFAULT_BUFFER_SIZE,
            };
            match unsafe { VirtioNetDriver::new(net_dev.mmio_base, virtio_cfg) } {
                Ok(mut driver) => {
                    download_with_config(&mut driver, download_config, None, tsc_freq)
                }
                Err(_) => {
                    puts("[BOOT] VirtIO driver init failed\n");
                    return RunResult::Failed;
                }
            }
        }
        NetDeviceType::IntelE1000e => {
            let intel_cfg = unsafe {
                E1000eConfig::new(dma_base, dma_bus, tsc_freq)
            };
            match unsafe { E1000eDriver::new(net_dev.mmio_base, intel_cfg) } {
                Ok(mut driver) => {
                    download_with_config(&mut driver, download_config, None, tsc_freq)
                }
                Err(_) => {
                    puts("[BOOT] Intel driver init failed\n");
                    return RunResult::Failed;
                }
            }
        }
    };

    match result {
        DownloadResult::Success { .. } => {
            puts("[BOOT] download complete!\n");
            RunResult::Success
        }
        DownloadResult::Failed { reason } => {
            puts("[BOOT] download failed: ");
            puts(reason);
            newline();
            RunResult::Failed
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// LEGACY COMPATIBILITY LAYER
// These types exist for gradual migration. Will be removed.
// ═══════════════════════════════════════════════════════════════════════════

/// NIC probe result (LEGACY - use hwinit instead).
#[derive(Debug, Clone, Copy)]
pub struct NicProbeResult {
    pub mmio_base: u64,
    pub pci_bus: u8,
    pub pci_device: u8,
    pub pci_function: u8,
    pub transport_type: u8,
    pub nic_type: u8,
    pub _pad: [u8; 3],
    pub common_cfg: u64,
    pub notify_cfg: u64,
    pub isr_cfg: u64,
    pub device_cfg: u64,
    pub notify_off_multiplier: u32,
    pub _pad2: u32,
}

pub const NIC_TYPE_NONE: u8 = 0;
pub const NIC_TYPE_VIRTIO: u8 = 1;
pub const NIC_TYPE_INTEL: u8 = 2;
pub const NIC_TYPE_REALTEK: u8 = 3;
pub const NIC_TYPE_BROADCOM: u8 = 4;

impl NicProbeResult {
    pub const fn zeroed() -> Self {
        Self {
            mmio_base: 0,
            pci_bus: 0,
            pci_device: 0,
            pci_function: 0,
            transport_type: 0,
            nic_type: NIC_TYPE_NONE,
            _pad: [0; 3],
            common_cfg: 0,
            notify_cfg: 0,
            isr_cfg: 0,
            device_cfg: 0,
            notify_off_multiplier: 0,
            _pad2: 0,
        }
    }

    pub const fn virtio_mmio(mmio_base: u64, bus: u8, device: u8, function: u8) -> Self {
        Self {
            mmio_base,
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            transport_type: 0,
            nic_type: NIC_TYPE_VIRTIO,
            _pad: [0; 3],
            common_cfg: 0,
            notify_cfg: 0,
            isr_cfg: 0,
            device_cfg: 0,
            notify_off_multiplier: 0,
            _pad2: 0,
        }
    }

    pub const fn mmio(mmio_base: u64, bus: u8, device: u8, function: u8) -> Self {
        Self::virtio_mmio(mmio_base, bus, device, function)
    }

    pub const fn pci_modern(
        common_cfg: u64,
        notify_cfg: u64,
        isr_cfg: u64,
        device_cfg: u64,
        notify_off_multiplier: u32,
        bus: u8,
        device: u8,
        function: u8,
    ) -> Self {
        Self {
            mmio_base: common_cfg,
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            transport_type: 1,
            nic_type: NIC_TYPE_VIRTIO,
            _pad: [0; 3],
            common_cfg,
            notify_cfg,
            isr_cfg,
            device_cfg,
            notify_off_multiplier,
            _pad2: 0,
        }
    }

    pub const fn intel(mmio_base: u64, bus: u8, device: u8, function: u8) -> Self {
        Self {
            mmio_base,
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            transport_type: 0,
            nic_type: NIC_TYPE_INTEL,
            _pad: [0; 3],
            common_cfg: 0,
            notify_cfg: 0,
            isr_cfg: 0,
            device_cfg: 0,
            notify_off_multiplier: 0,
            _pad2: 0,
        }
    }
}

/// Block device probe result (LEGACY - use hwinit instead).
#[derive(Debug, Clone, Copy)]
pub struct BlkProbeResult {
    pub mmio_base: u64,
    pub pci_bus: u8,
    pub pci_device: u8,
    pub pci_function: u8,
    pub device_type: u8,
    pub transport_type: u8,
    pub _pad: [u8; 3],
    pub sector_size: u32,
    pub total_sectors: u64,
    pub common_cfg: u64,
    pub notify_cfg: u64,
    pub notify_off_multiplier: u32,
    pub isr_cfg: u64,
    pub device_cfg: u64,
}

impl BlkProbeResult {
    pub const fn zeroed() -> Self {
        Self {
            mmio_base: 0,
            pci_bus: 0,
            pci_device: 0,
            pci_function: 0,
            device_type: 0,
            transport_type: 0,
            _pad: [0; 3],
            sector_size: 512,
            total_sectors: 0,
            common_cfg: 0,
            notify_cfg: 0,
            notify_off_multiplier: 0,
            isr_cfg: 0,
            device_cfg: 0,
        }
    }

    /// Create AHCI result.
    pub const fn ahci(abar: u64, bus: u8, device: u8, function: u8) -> Self {
        Self {
            mmio_base: abar,
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            device_type: 3, // AHCI
            transport_type: 0,
            _pad: [0; 3],
            sector_size: 512,
            total_sectors: 0,
            common_cfg: 0,
            notify_cfg: 0,
            notify_off_multiplier: 0,
            isr_cfg: 0,
            device_cfg: 0,
        }
    }

    /// Create VirtIO-blk result.
    pub const fn virtio(mmio_base: u64, bus: u8, device: u8, function: u8) -> Self {
        Self {
            mmio_base,
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            device_type: 1, // VirtIO
            transport_type: 0,
            _pad: [0; 3],
            sector_size: 512,
            total_sectors: 0,
            common_cfg: 0,
            notify_cfg: 0,
            notify_off_multiplier: 0,
            isr_cfg: 0,
            device_cfg: 0,
        }
    }

    /// Create VirtIO-blk result for PCI Modern.
    pub const fn pci_modern(
        common_cfg: u64,
        notify_cfg: u64,
        isr_cfg: u64,
        device_cfg: u64,
        notify_off_multiplier: u32,
        bus: u8,
        device: u8,
        function: u8,
    ) -> Self {
        Self {
            mmio_base: 0,
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            device_type: 1, // VirtIO
            transport_type: 1, // PCI Modern
            _pad: [0; 3],
            sector_size: 512,
            total_sectors: 0,
            common_cfg,
            notify_cfg,
            notify_off_multiplier,
            isr_cfg,
            device_cfg,
        }
    }
}

// LEGACY functions - will be removed once bootloader migrates
pub fn prepare_handoff(
    _nic: &NicProbeResult,
    _mac_address: [u8; 6],
    _dma_cpu_ptr: u64,
    _dma_bus_addr: u64,
    _dma_size: u64,
    _tsc_freq: u64,
    _stack_top: u64,
    _stack_size: u64,
) -> BootHandoff {
    // Stub - bootloader should migrate to enter_baremetal_download
    panic!("Legacy prepare_handoff called - migrate to new architecture");
}

pub fn prepare_handoff_with_blk(
    _nic: &NicProbeResult,
    _blk: &BlkProbeResult,
    _mac_address: [u8; 6],
    _dma_cpu_ptr: u64,
    _dma_bus_addr: u64,
    _dma_size: u64,
    _tsc_freq: u64,
    _stack_top: u64,
    _stack_size: u64,
) -> BootHandoff {
    panic!("Legacy prepare_handoff_with_blk called - migrate to new architecture");
}

pub fn prepare_handoff_full(
    _nic: &NicProbeResult,
    _blk: &BlkProbeResult,
    _mac_address: [u8; 6],
    _dma_cpu_ptr: u64,
    _dma_bus_addr: u64,
    _dma_size: u64,
    _tsc_freq: u64,
    _stack_top: u64,
    _stack_size: u64,
    _fb_base: u64,
    _fb_width: u32,
    _fb_height: u32,
    _fb_stride: u32,
    _fb_format: u32,
) -> BootHandoff {
    panic!("Legacy prepare_handoff_full called - migrate to new architecture");
}

pub fn validate_handoff(_handoff: &BootHandoff) -> bool {
    false
}

/// LEGACY: Old entry point wrapper (routes to new implementation)
pub unsafe fn enter_network_boot(_handoff: &'static BootHandoff) -> RunResult {
    panic!("Legacy enter_network_boot called - use enter_baremetal_download");
}

/// LEGACY: Old entry point wrapper with URL
pub unsafe fn enter_network_boot_url(
    _handoff: &'static BootHandoff,
    _iso_url: &'static str,
    _esp_lba: u64,
) -> RunResult {
    panic!("Legacy enter_network_boot_url called - use enter_baremetal_download");
}
