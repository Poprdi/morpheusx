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
    let config = BareMetalConfig {
        iso_url,
        ..BareMetalConfig::default()
    };
    
    bare_metal_main(handoff, config)
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
    use morpheus_network::boot::handoff::{
        HANDOFF_MAGIC, HANDOFF_VERSION, NIC_TYPE_VIRTIO, BLK_TYPE_NONE,
    };
    
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
        
        blk_mmio_base: 0,
        blk_pci_bus: 0,
        blk_pci_device: 0,
        blk_pci_function: 0,
        blk_type: BLK_TYPE_NONE,
        blk_sector_size: 512,
        blk_total_sectors: 0,
        
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
        
        // PCI Modern transport fields
        nic_transport_type: nic.transport_type,
        _transport_pad: [0; 3],
        nic_notify_off_multiplier: nic.notify_off_multiplier,
        nic_common_cfg: nic.common_cfg,
        nic_notify_cfg: nic.notify_cfg,
        nic_isr_cfg: nic.isr_cfg,
        nic_device_cfg: nic.device_cfg,
        
        _reserved: [0; 8],
    }
}

/// Test if network boot handoff is valid.
pub fn validate_handoff(handoff: &BootHandoff) -> bool {
    handoff.validate().is_ok()
}
