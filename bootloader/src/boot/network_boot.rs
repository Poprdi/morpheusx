//! Network boot integration for post-ExitBootServices ISO download.
//!
//! This module bridges the UEFI bootloader and the bare-metal network stack.
//!
//! # Flow
//! 1. Bootloader runs pre-EBS: probes hardware, allocates DMA, calibrates TSC
//! 2. Bootloader calls ExitBootServices
//! 3. Bootloader calls `enter_network_boot()` with BootHandoff
//! 4. Network stack downloads ISO and writes to disk
//! 5. Control returns for OS boot from downloaded ISO

#![allow(dead_code)]
#![allow(unused_imports)]

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::ToString;

use morpheus_network::boot::handoff::BootHandoff;
use morpheus_network::mainloop::{bare_metal_main, BareMetalConfig, RunResult};

/// Network boot entry point (post-EBS).
///
/// # Safety
/// - Must be called after ExitBootServices()
/// - `handoff` must point to valid, populated BootHandoff
/// - Must be on pre-allocated stack
pub unsafe fn enter_network_boot(handoff: &'static BootHandoff) -> RunResult {
    // Default config: download from QEMU host HTTP server
    let config = BareMetalConfig::default();

    bare_metal_main(handoff, config)
}

/// Network boot with custom URL.
///
/// # Safety
/// - Must be called after ExitBootServices()
/// - `handoff` must point to valid, populated BootHandoff
/// - `iso_url` must be a 'static str (allocated before EBS, e.g., via Box::leak)
pub unsafe fn enter_network_boot_url(
    handoff: &'static BootHandoff,
    iso_url: &'static str,
) -> RunResult {
    // Extract ISO filename from URL (e.g., "tails-amd64-7.3.1.iso" from full URL)
    let iso_name = extract_iso_name_from_url(iso_url);

    let config = BareMetalConfig {
        iso_url,
        iso_name,
        ..BareMetalConfig::default()
    };

    bare_metal_main(handoff, config)
}

/// Extract ISO filename from URL.
/// Returns the last path component, or "download.iso" if none found.
fn extract_iso_name_from_url(url: &str) -> &'static str {
    // Find the last '/' in the URL
    if let Some(pos) = url.rfind('/') {
        let filename = &url[pos + 1..];
        // Check if it looks like an ISO filename
        if filename.len() > 4 && (filename.ends_with(".iso") || filename.ends_with(".ISO")) {
            // Leak the extracted filename to make it 'static
            // This is safe since we're in bare-metal mode and won't free memory
            let leaked: &'static str = Box::leak(filename.to_string().into_boxed_str());
            return leaked;
        }
    }
    "download.iso"
}

/// NIC probe result with transport information.
#[derive(Debug, Clone, Copy)]
pub struct NicProbeResult {
    /// MMIO base address (for legacy, or device_cfg for PCI modern)
    pub mmio_base: u64,
    /// PCI bus number
    pub pci_bus: u8,
    /// PCI device number
    pub pci_device: u8,
    /// PCI function number
    pub pci_function: u8,
    /// Transport type: 0=MMIO, 1=PCI Modern, 2=PCI Legacy
    pub transport_type: u8,
    /// Common cfg address (PCI Modern only)
    pub common_cfg: u64,
    /// Notify cfg address (PCI Modern only)
    pub notify_cfg: u64,
    /// ISR cfg address (PCI Modern only)
    pub isr_cfg: u64,
    /// Device cfg address (PCI Modern only)
    pub device_cfg: u64,
    /// Notify offset multiplier (PCI Modern only)
    pub notify_off_multiplier: u32,
}

impl NicProbeResult {
    /// Create a new zeroed probe result.
    pub const fn zeroed() -> Self {
        Self {
            mmio_base: 0,
            pci_bus: 0,
            pci_device: 0,
            pci_function: 0,
            transport_type: 0,
            common_cfg: 0,
            notify_cfg: 0,
            isr_cfg: 0,
            device_cfg: 0,
            notify_off_multiplier: 0,
        }
    }

    /// Create MMIO transport result.
    pub const fn mmio(mmio_base: u64, bus: u8, device: u8, function: u8) -> Self {
        Self {
            mmio_base,
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            transport_type: 0, // TRANSPORT_MMIO
            common_cfg: 0,
            notify_cfg: 0,
            isr_cfg: 0,
            device_cfg: 0,
            notify_off_multiplier: 0,
        }
    }

    /// Create PCI Modern transport result.
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
            mmio_base: common_cfg, // Use common_cfg as mmio_base for PCI Modern
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            transport_type: 1, // TRANSPORT_PCI_MODERN
            common_cfg,
            notify_cfg,
            isr_cfg,
            device_cfg,
            notify_off_multiplier,
        }
    }
}

/// Block device probe result.
#[derive(Debug, Clone, Copy)]
pub struct BlkProbeResult {
    /// MMIO base address (legacy) or 0 for PCI Modern
    pub mmio_base: u64,
    /// PCI bus number
    pub pci_bus: u8,
    /// PCI device number
    pub pci_device: u8,
    /// PCI function number
    pub pci_function: u8,
    /// Device type: 0=None, 1=VirtIO-blk
    pub device_type: u8,
    /// Transport type: 0=MMIO, 1=PCI Modern, 2=PCI Legacy
    pub transport_type: u8,
    /// Padding
    pub _pad: [u8; 3],
    /// Sector size (typically 512)
    pub sector_size: u32,
    /// Total sectors
    pub total_sectors: u64,
    /// PCI Modern: common_cfg address
    pub common_cfg: u64,
    /// PCI Modern: notify_cfg address
    pub notify_cfg: u64,
    /// PCI Modern: notify offset multiplier
    pub notify_off_multiplier: u32,
    /// PCI Modern: isr_cfg address
    pub isr_cfg: u64,
    /// PCI Modern: device_cfg address
    pub device_cfg: u64,
}

impl BlkProbeResult {
    /// Create a new zeroed probe result (no device found).
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

    /// Create VirtIO-blk result (legacy MMIO).
    pub const fn virtio(mmio_base: u64, bus: u8, device: u8, function: u8) -> Self {
        Self {
            mmio_base,
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            device_type: 1,    // BLK_TYPE_VIRTIO
            transport_type: 0, // MMIO
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

    /// Create VirtIO-blk result (PCI Modern).
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
            mmio_base: 0, // No legacy MMIO for PCI Modern
            pci_bus: bus,
            pci_device: device,
            pci_function: function,
            device_type: 1,    // BLK_TYPE_VIRTIO
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

/// Prepare BootHandoff from UEFI boot services.
///
/// Call this BEFORE ExitBootServices to populate handoff structure.
pub fn prepare_handoff(
    nic: &NicProbeResult,
    mac_address: [u8; 6],
    dma_cpu_ptr: u64,
    dma_bus_addr: u64,
    dma_size: u64,
    tsc_freq: u64,
    stack_top: u64,
    stack_size: u64,
) -> BootHandoff {
    // Delegate to full version with no block device
    prepare_handoff_with_blk(
        nic,
        &BlkProbeResult::zeroed(),
        mac_address,
        dma_cpu_ptr,
        dma_bus_addr,
        dma_size,
        tsc_freq,
        stack_top,
        stack_size,
    )
}

/// Prepare BootHandoff with both NIC and block device info.
///
/// Call this BEFORE ExitBootServices to populate handoff structure.
pub fn prepare_handoff_with_blk(
    nic: &NicProbeResult,
    blk: &BlkProbeResult,
    mac_address: [u8; 6],
    dma_cpu_ptr: u64,
    dma_bus_addr: u64,
    dma_size: u64,
    tsc_freq: u64,
    stack_top: u64,
    stack_size: u64,
) -> BootHandoff {
    use morpheus_network::boot::handoff::{HANDOFF_MAGIC, HANDOFF_VERSION, NIC_TYPE_VIRTIO};

    BootHandoff {
        magic: HANDOFF_MAGIC,
        version: HANDOFF_VERSION,
        size: core::mem::size_of::<BootHandoff>() as u32,

        nic_mmio_base: nic.mmio_base,
        nic_pci_bus: nic.pci_bus,
        nic_pci_device: nic.pci_device,
        nic_pci_function: nic.pci_function,
        nic_type: NIC_TYPE_VIRTIO,
        mac_address,
        _nic_pad: [0; 2],

        blk_mmio_base: blk.mmio_base,
        blk_pci_bus: blk.pci_bus,
        blk_pci_device: blk.pci_device,
        blk_pci_function: blk.pci_function,
        blk_type: blk.device_type,
        blk_sector_size: blk.sector_size,
        blk_total_sectors: blk.total_sectors,

        dma_cpu_ptr,
        dma_bus_addr,
        dma_size,

        tsc_freq,

        stack_top,
        stack_size,

        framebuffer_base: 0,
        framebuffer_width: 0,
        framebuffer_height: 0,
        framebuffer_stride: 0,
        framebuffer_format: 0,

        memory_map_ptr: 0,
        memory_map_size: 0,
        memory_map_desc_size: 0,

        // PCI Modern transport fields (NIC)
        nic_transport_type: nic.transport_type,
        _transport_pad: [0; 3],
        nic_notify_off_multiplier: nic.notify_off_multiplier,
        nic_common_cfg: nic.common_cfg,
        nic_notify_cfg: nic.notify_cfg,
        nic_isr_cfg: nic.isr_cfg,
        nic_device_cfg: nic.device_cfg,

        // PCI Modern transport fields (BLK)
        blk_transport_type: blk.transport_type,
        _blk_transport_pad: [0; 3],
        blk_notify_off_multiplier: blk.notify_off_multiplier,
        blk_common_cfg: blk.common_cfg,
        blk_notify_cfg: blk.notify_cfg,
        blk_isr_cfg: blk.isr_cfg,
        blk_device_cfg: blk.device_cfg,

        _reserved: [0; 8],
    }
}

/// Test if network boot handoff is valid.
pub fn validate_handoff(handoff: &BootHandoff) -> bool {
    handoff.validate().is_ok()
}
