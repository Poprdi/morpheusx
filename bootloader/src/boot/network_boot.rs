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

/// Prepare BootHandoff from UEFI boot services.
///
/// Call this BEFORE ExitBootServices to populate handoff structure.
pub fn prepare_handoff(
    nic_mmio_base: u64,
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
        
        nic_mmio_base,
        nic_pci_bus: 0,
        nic_pci_device: 3,
        nic_pci_function: 0,
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
        
        _reserved: [0; 56],
    }
}

/// Test if network boot handoff is valid.
pub fn validate_handoff(handoff: &BootHandoff) -> bool {
    handoff.validate().is_ok()
}
