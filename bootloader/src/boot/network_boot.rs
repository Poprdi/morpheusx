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
    // Legacy entry
    platform_init, PlatformConfig,
    // New self-contained entry (recommended)
    platform_init_selfcontained, SelfContainedConfig,
    // Common types (platform only - no device types)
    InitError, PlatformInit,
};
use morpheus_network::boot::handoff::BootHandoff;
use morpheus_network::boot::probe::{probe_network_device, scan_for_nic, DetectedNic, ProbeResult, ProbeError};
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
                InitError::MemoryRegistryFailed => puts("memory registry init failed"),
                InitError::GdtInitFailed => puts("GDT init failed"),
                InitError::IdtInitFailed => puts("IDT init failed"),
                InitError::PicInitFailed => puts("PIC init failed"),
                InitError::HeapInitFailed => puts("heap init failed"),
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
// SELF-CONTAINED BARE-METAL ENTRY (RECOMMENDED)
// ═══════════════════════════════════════════════════════════════════════════

/// Configuration for bare-metal entry after ExitBootServices.
///
/// Only needs the UEFI memory map - hwinit handles everything else.
pub struct BaremetalEntryConfig {
    /// Pointer to UEFI memory map (from ExitBootServices)
    pub memory_map_ptr: *const u8,
    /// Size of memory map in bytes
    pub memory_map_size: usize,
    /// Size of each descriptor entry
    pub descriptor_size: usize,
    /// Descriptor version (from UEFI)
    pub descriptor_version: u32,
}

/// Download request for bare-metal mode.
pub struct DownloadRequest {
    /// URL to download
    pub url: &'static str,
    /// Name for manifest
    pub name: &'static str,
    /// ESP start LBA for disk writes (0 = don't persist)
    pub esp_start_lba: u64,
}

/// Result of bare-metal operations.
#[derive(Debug, Clone, Copy)]
pub enum BaremetalResult {
    /// Download completed successfully
    DownloadComplete { bytes: u64 },
    /// Download failed
    DownloadFailed,
    /// No network device found
    NoNetworkDevice,
    /// Platform init failed
    PlatformInitFailed,
}

/// Bare-metal main entry point.
///
/// This is THE entry point after ExitBootServices. We never return to UEFI.
///
/// # Flow
/// 1. hwinit takes ownership (GDT, IDT, PIC, heap, DMA, PCI)
/// 2. Execute the requested download
/// 3. Return to bare-metal main loop (for now: halt, later: menu)
///
/// # Safety
/// - Must be called IMMEDIATELY after ExitBootServices
/// - Memory map must be valid
/// - NEVER returns to UEFI - we own the machine now
pub unsafe fn enter_baremetal_world(
    config: BaremetalEntryConfig,
    download: DownloadRequest,
) -> ! {
    use morpheus_hwinit::serial::{puts, put_hex64, newline};

    puts("\n");
    puts("╔══════════════════════════════════════════════════════════════╗\n");
    puts("║              MORPHEUS BARE-METAL MODE                        ║\n");
    puts("║              UEFI is gone. We own the machine.               ║\n");
    puts("╚══════════════════════════════════════════════════════════════╝\n");
    puts("\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 1: hwinit takes ownership
    // ─────────────────────────────────────────────────────────────────────

    let hwinit_config = SelfContainedConfig {
        memory_map_ptr: config.memory_map_ptr,
        memory_map_size: config.memory_map_size,
        descriptor_size: config.descriptor_size,
        descriptor_version: config.descriptor_version,
    };

    let platform = match platform_init_selfcontained(hwinit_config) {
        Ok(p) => p,
        Err(e) => {
            puts("[BAREMETAL] FATAL: platform init failed: ");
            match e {
                InitError::InvalidDmaRegion => puts("invalid DMA region"),
                InitError::NoDevicesFound => puts("no devices found"),
                InitError::BarDecodeFailed => puts("BAR decode failed"),
                InitError::TscCalibrationFailed => puts("TSC calibration failed"),
                InitError::NoFreeMemory => puts("no free memory"),
                InitError::MemoryRegistryFailed => puts("memory registry init failed"),
                InitError::GdtInitFailed => puts("GDT init failed"),
                InitError::IdtInitFailed => puts("IDT init failed"),
                InitError::PicInitFailed => puts("PIC init failed"),
                InitError::HeapInitFailed => puts("heap init failed"),
            }
            newline();
            baremetal_halt("Platform initialization failed");
        }
    };

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 2: Execute download
    // ─────────────────────────────────────────────────────────────────────

    let result = execute_download(&platform, &download);

    match result {
        BaremetalResult::DownloadComplete { bytes } => {
            puts("\n");
            puts("╔══════════════════════════════════════════════════════════════╗\n");
            puts("║                    DOWNLOAD COMPLETE                         ║\n");
            puts("╚══════════════════════════════════════════════════════════════╝\n");
            puts("[BAREMETAL] Bytes written: ");
            put_hex64(bytes);
            newline();
        }
        BaremetalResult::DownloadFailed => {
            puts("[BAREMETAL] Download failed\n");
        }
        BaremetalResult::NoNetworkDevice => {
            puts("[BAREMETAL] No network device found\n");
        }
        BaremetalResult::PlatformInitFailed => {
            puts("[BAREMETAL] Platform init failed\n");
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 3: Bare-metal main loop
    // ─────────────────────────────────────────────────────────────────────
    // TODO: This will become a proper menu once display is sorted
    // For now, just report completion and wait

    baremetal_main_loop(&platform);
}

/// Execute a download in bare-metal mode.
unsafe fn execute_download(platform: &PlatformInit, download: &DownloadRequest) -> BaremetalResult {
    use morpheus_hwinit::serial::{puts, put_hex64, newline};

    // Find network device
    let net_dev = match platform.net_devices.iter().find_map(|d| *d) {
        Some(d) => d,
        None => {
            return BaremetalResult::NoNetworkDevice;
        }
    };

    puts("[BAREMETAL] Network device: ");
    match net_dev.device_type {
        NetDeviceType::VirtIO => puts("VirtIO-net"),
        NetDeviceType::IntelE1000e => puts("Intel e1000e"),
    }
    puts(" @ ");
    put_hex64(net_dev.mmio_base);
    newline();

    // Build download config
    let download_config = DownloadConfig {
        url: download.url,
        write_to_disk: download.esp_start_lba > 0,
        target_start_sector: 0,
        manifest_sector: 0,
        esp_start_lba: download.esp_start_lba,
        partition_uuid: [0u8; 16],
        iso_name: download.name,
        expected_size: 0,
    };

    let dma_cpu = platform.dma_region.cpu_base();
    let dma_bus = platform.dma_region.bus_base();
    let dma_size = platform.dma_region.size();
    let tsc_freq = platform.tsc_freq;

    // Create driver and execute download
    let result = match net_dev.device_type {
        NetDeviceType::VirtIO => {
            let virtio_cfg = VirtioConfig {
                dma_cpu_base: dma_cpu,
                dma_bus_base: dma_bus,
                dma_size,
                queue_size: VirtioConfig::DEFAULT_QUEUE_SIZE,
                buffer_size: VirtioConfig::DEFAULT_BUFFER_SIZE,
            };
            match VirtioNetDriver::new(net_dev.mmio_base, virtio_cfg) {
                Ok(mut driver) => {
                    download_with_config(&mut driver, download_config, None, tsc_freq)
                }
                Err(_) => {
                    puts("[BAREMETAL] VirtIO driver init failed\n");
                    return BaremetalResult::DownloadFailed;
                }
            }
        }
        NetDeviceType::IntelE1000e => {
            let intel_cfg = E1000eConfig::new(dma_cpu, dma_bus, tsc_freq);
            match E1000eDriver::new(net_dev.mmio_base, intel_cfg) {
                Ok(mut driver) => {
                    download_with_config(&mut driver, download_config, None, tsc_freq)
                }
                Err(_) => {
                    puts("[BAREMETAL] Intel driver init failed\n");
                    return BaremetalResult::DownloadFailed;
                }
            }
        }
    };

    match result {
        DownloadResult::Success { bytes_written, .. } => {
            BaremetalResult::DownloadComplete { bytes: bytes_written as u64 }
        }
        DownloadResult::Failed { reason } => {
            puts("[BAREMETAL] Download failed: ");
            puts(reason);
            newline();
            BaremetalResult::DownloadFailed
        }
    }
}

/// Bare-metal main loop.
///
/// This is where we live after UEFI is gone. For now it's a placeholder
/// that just waits. Eventually this will be a full menu system with
/// our own display driver.
fn baremetal_main_loop(_platform: &PlatformInit) -> ! {
    use morpheus_hwinit::serial::puts;

    puts("\n");
    puts("╔══════════════════════════════════════════════════════════════╗\n");
    puts("║              BARE-METAL MAIN LOOP                            ║\n");
    puts("║                                                              ║\n");
    puts("║  UEFI is gone. We own the machine.                           ║\n");
    puts("║                                                              ║\n");
    puts("║  TODO: Display driver, menu system, boot selection           ║\n");
    puts("║                                                              ║\n");
    puts("║  For now: System ready. Waiting for next command...          ║\n");
    puts("╚══════════════════════════════════════════════════════════════╝\n");
    puts("\n");

    // TODO: Eventually this will be:
    // 1. Display bare-metal menu (using our framebuffer driver)
    // 2. Handle input (keyboard driver)
    // 3. Boot Linux (kexec-style)
    // 4. Or download another ISO
    // 5. Or configure settings
    //
    // For now, just spin. The machine is ours.

    loop {
        core::hint::spin_loop();
    }
}

/// Halt with message (for fatal errors).
fn baremetal_halt(msg: &str) -> ! {
    use morpheus_hwinit::serial::puts;
    puts("[BAREMETAL] HALT: ");
    puts(msg);
    puts("\n");
    loop { core::hint::spin_loop(); }
}

// Keep the old struct for backward compatibility during migration
/// Configuration for self-contained bare-metal download.
/// DEPRECATED: Use BaremetalEntryConfig + DownloadRequest instead.
pub struct SelfContainedDownloadConfig {
    pub memory_map_ptr: *const u8,
    pub memory_map_size: usize,
    pub descriptor_size: usize,
    pub descriptor_version: u32,
    pub iso_url: &'static str,
    pub iso_name: &'static str,
    pub esp_start_lba: u64,
}

/// DEPRECATED: Use enter_baremetal_world instead.
pub unsafe fn enter_selfcontained_download(config: SelfContainedDownloadConfig) -> ! {
    enter_baremetal_world(
        BaremetalEntryConfig {
            memory_map_ptr: config.memory_map_ptr,
            memory_map_size: config.memory_map_size,
            descriptor_size: config.descriptor_size,
            descriptor_version: config.descriptor_version,
        },
        DownloadRequest {
            url: config.iso_url,
            name: config.iso_name,
            esp_start_lba: config.esp_start_lba,
        },
    )
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
