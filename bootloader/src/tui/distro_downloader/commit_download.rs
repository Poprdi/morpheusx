//! Download commit flow - transitions from UEFI to bare-metal for ISO download.
//!
//! This module handles the critical ExitBootServices transition:
//! 1. User has selected ISO from catalog and confirmed download
//! 2. This module prepares the BootHandoff structure
//! 3. Calibrates TSC timing
//! 4. Calls ExitBootServices (POINT OF NO RETURN)
//! 5. Jumps to bare_metal_main for actual download
//!
//! # Safety
//! After ExitBootServices:
//! - No UEFI runtime services for networking
//! - No heap allocator (must pre-allocate)
//! - No screen/console output (use serial)
//! - Must use pre-allocated stack

extern crate alloc;

use alloc::string::String;
use core::ptr;

use crate::tui::renderer::{Screen, EFI_BLACK, EFI_LIGHTGREEN, EFI_YELLOW, EFI_RED, EFI_CYAN};
use crate::boot::network_boot::{prepare_handoff, enter_network_boot_url};
use morpheus_network::boot::handoff::BootHandoff;

/// Result of download commit operation.
#[derive(Debug)]
pub enum CommitResult {
    /// Download completed successfully, system should reboot
    Success,
    /// Download failed, error message available
    Failed(&'static str),
    /// User cancelled before commit
    Cancelled,
}

/// Configuration for download commit.
pub struct DownloadCommitConfig {
    /// URL to download from
    pub iso_url: String,
    /// Expected size in bytes (for progress)
    pub iso_size: u64,
    /// Name of the distro (for display)
    pub distro_name: String,
}

/// UEFI memory types (from UEFI spec)
const EFI_LOADER_DATA: usize = 2;
const EFI_ALLOCATE_MAX_ADDRESS: usize = 1;
const EFI_ALLOCATE_ANY_PAGES: usize = 0;

/// Commit to download - this exits boot services and downloads in bare-metal mode.
///
/// # POINT OF NO RETURN
/// Once this function calls ExitBootServices, there's no going back.
/// The system will either:
/// 1. Download successfully and reboot
/// 2. Fail and panic (requires power cycle)
///
/// # Arguments
/// * `boot_services` - Pointer to UEFI boot services table
/// * `image_handle` - UEFI image handle
/// * `screen` - Screen for status display (unusable after EBS)
/// * `config` - Download configuration
///
/// # Safety
/// This function will never return on success. On failure, it loops forever.
pub unsafe fn commit_to_download(
    boot_services: *const crate::BootServices,
    image_handle: *mut (),
    screen: &mut Screen,
    config: DownloadCommitConfig,
) -> ! {
    let bs = &*boot_services;
    
    // Display countdown and status
    display_commit_countdown(screen, &config, bs);
    
    // Phase 1: Allocate DMA region (must be <4GB for VirtIO)
    screen.clear();
    screen.put_str_at(5, 2, "=== Preparing Download Environment ===", EFI_LIGHTGREEN, EFI_BLACK);
    let mut log_y = 4;
    
    screen.put_str_at(5, log_y, "Allocating DMA region...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    
    const DMA_SIZE: usize = 8 * 1024 * 1024; // 8MB DMA pool
    const DMA_PAGES: usize = DMA_SIZE / 4096;
    
    let mut dma_region: u64 = 0xFFFF_FFFF; // Max address hint
    let status = (bs.allocate_pages)(
        EFI_ALLOCATE_MAX_ADDRESS,
        EFI_LOADER_DATA,
        DMA_PAGES,
        &mut dma_region,
    );
    
    if status != 0 {
        screen.put_str_at(7, log_y, "DMA allocation failed!", EFI_RED, EFI_BLACK);
        log_y += 1;
        screen.put_str_at(7, log_y, "Cannot proceed with download", EFI_RED, EFI_BLACK);
        // Hang forever
        loop { core::hint::spin_loop(); }
    }
    
    // Zero the DMA region
    ptr::write_bytes(dma_region as *mut u8, 0, DMA_SIZE);
    
    screen.put_str_at(7, log_y, &alloc::format!("DMA base: {:#x}", dma_region), EFI_CYAN, EFI_BLACK);
    log_y += 2;
    
    // Phase 2: Allocate stack for bare-metal mode
    screen.put_str_at(5, log_y, "Allocating bare-metal stack...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    
    const STACK_SIZE: usize = 256 * 1024; // 256KB stack
    const STACK_PAGES: usize = STACK_SIZE / 4096;
    
    let mut stack_region: u64 = 0;
    let status = (bs.allocate_pages)(
        EFI_ALLOCATE_ANY_PAGES,
        EFI_LOADER_DATA,
        STACK_PAGES,
        &mut stack_region,
    );
    
    if status != 0 {
        screen.put_str_at(7, log_y, "Stack allocation failed!", EFI_RED, EFI_BLACK);
        loop { core::hint::spin_loop(); }
    }
    
    let stack_top = stack_region + STACK_SIZE as u64; // Stack grows down
    screen.put_str_at(7, log_y, &alloc::format!("Stack: {:#x}", stack_region), EFI_CYAN, EFI_BLACK);
    log_y += 2;
    
    // Phase 3: Calibrate TSC using UEFI Stall
    screen.put_str_at(5, log_y, "Calibrating TSC timing...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    
    let tsc_freq = calibrate_tsc_with_stall(bs);
    screen.put_str_at(7, log_y, &alloc::format!("TSC: {} Hz", tsc_freq), EFI_CYAN, EFI_BLACK);
    log_y += 2;
    
    // Phase 4: Probe VirtIO NIC via PCI (just finding it, not initializing)
    screen.put_str_at(5, log_y, "Probing VirtIO network device...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    
    let (nic_base, is_io_space) = probe_virtio_nic_with_debug(screen, &mut log_y);
    if nic_base == 0 {
        screen.put_str_at(7, log_y, "Ensure QEMU has: -device virtio-net-pci,netdev=...", EFI_RED, EFI_BLACK);
        loop { core::hint::spin_loop(); }
    }
    
    // Note: We store the base address regardless of type (IO vs MMIO)
    // The actual driver init happens post-EBS
    let nic_mmio_base = nic_base;
    log_y += 1;
    
    // Phase 5: Prepare BootHandoff (static allocation for post-EBS)
    screen.put_str_at(5, log_y, "Preparing boot handoff...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    
    // Allocate handoff in loader data so it survives EBS
    let mut handoff_page: u64 = 0;
    let status = (bs.allocate_pages)(
        EFI_ALLOCATE_ANY_PAGES,
        EFI_LOADER_DATA,
        1, // 4KB is plenty
        &mut handoff_page,
    );
    
    if status != 0 {
        screen.put_str_at(7, log_y, "Handoff allocation failed!", EFI_RED, EFI_BLACK);
        loop { core::hint::spin_loop(); }
    }
    
    let handoff_ptr = handoff_page as *mut BootHandoff;
    
    let handoff = prepare_handoff(
        nic_mmio_base,
        [0x52, 0x54, 0x00, 0x12, 0x34, 0x56], // QEMU default MAC (placeholder)
        dma_region,
        dma_region, // Bus addr = CPU addr (no IOMMU)
        DMA_SIZE as u64,
        tsc_freq,
        stack_top,
        STACK_SIZE as u64,
    );
    
    ptr::write(handoff_ptr, handoff);
    let handoff_ref: &'static BootHandoff = &*handoff_ptr;
    
    screen.put_str_at(7, log_y, "Handoff ready", EFI_CYAN, EFI_BLACK);
    log_y += 2;
    
    // Phase 6: Store URL for bare-metal use
    let url_copy = leak_string(&config.iso_url);
    
    // Phase 7: Final countdown before EBS
    screen.put_str_at(5, log_y, "=== EXITING BOOT SERVICES ===", EFI_RED, EFI_BLACK);
    log_y += 1;
    screen.put_str_at(5, log_y, "After this point:", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    screen.put_str_at(7, log_y, "- Screen output will stop", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    screen.put_str_at(7, log_y, "- Progress via serial console only", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    screen.put_str_at(7, log_y, "- System will reboot when done", EFI_YELLOW, EFI_BLACK);
    log_y += 2;
    
    // Brief pause for user to see message
    for _ in 0..200_000_000u64 { core::hint::spin_loop(); }
    
    screen.put_str_at(5, log_y, "Exiting boot services NOW...", EFI_RED, EFI_BLACK);
    
    // ═══════════════════════════════════════════════════════════════════════
    // POINT OF NO RETURN - EXIT BOOT SERVICES
    // ═══════════════════════════════════════════════════════════════════════
    
    // Get memory map first
    let mut mmap_size: usize = 4096;
    let mut mmap_buf = [0u8; 8192]; // Large enough buffer
    let mut map_key: usize = 0;
    let mut desc_size: usize = 0;
    let mut desc_version: u32 = 0;
    
    // First call to get required size
    let _ = (bs.get_memory_map)(
        &mut mmap_size,
        mmap_buf.as_mut_ptr(),
        &mut map_key,
        &mut desc_size,
        &mut desc_version,
    );
    
    // Increase buffer size to be safe
    mmap_size += 1024;
    
    // Second call with proper size
    let status = (bs.get_memory_map)(
        &mut mmap_size,
        mmap_buf.as_mut_ptr(),
        &mut map_key,
        &mut desc_size,
        &mut desc_version,
    );
    
    if status != 0 {
        // Cannot display error properly at this point
        loop { core::hint::spin_loop(); }
    }
    
    // Exit boot services - MUST succeed
    let status = (bs.exit_boot_services)(image_handle, map_key);
    
    if status != 0 {
        // Fatal - cannot recover, memory map may have changed
        // Try once more with fresh map
        let _ = (bs.get_memory_map)(
            &mut mmap_size,
            mmap_buf.as_mut_ptr(),
            &mut map_key,
            &mut desc_size,
            &mut desc_version,
        );
        let status = (bs.exit_boot_services)(image_handle, map_key);
        if status != 0 {
            loop { core::hint::spin_loop(); }
        }
    }
    
    // ═══════════════════════════════════════════════════════════════════════
    // WE ARE NOW IN BARE-METAL MODE
    // - No UEFI services available
    // - Must use serial for output  
    // - Must use our own drivers
    // ═══════════════════════════════════════════════════════════════════════
    
    // Enter bare-metal download
    let _result = enter_network_boot_url(handoff_ref, url_copy);
    
    // If we get here, download completed (success or failure)
    // In bare-metal mode, we can't return to UEFI - must reset
    loop { core::hint::spin_loop(); }
}

/// Display countdown before committing to download.
fn display_commit_countdown(screen: &mut Screen, config: &DownloadCommitConfig, bs: &crate::BootServices) {
    screen.clear();
    
    screen.put_str_at(5, 2, "=== Download Confirmation ===", EFI_LIGHTGREEN, EFI_BLACK);
    
    screen.put_str_at(5, 4, "About to download:", EFI_YELLOW, EFI_BLACK);
    screen.put_str_at(7, 5, &config.distro_name, EFI_CYAN, EFI_BLACK);
    screen.put_str_at(7, 6, &alloc::format!("Size: {} MB", config.iso_size / (1024 * 1024)), EFI_CYAN, EFI_BLACK);
    
    screen.put_str_at(5, 8, "WARNING: This will exit UEFI boot services!", EFI_RED, EFI_BLACK);
    screen.put_str_at(5, 9, "The system cannot be interrupted during download.", EFI_RED, EFI_BLACK);
    
    // Use UEFI Stall for accurate 1-second delays (1_000_000 microseconds = 1 second)
    screen.put_str_at(5, 11, "Starting in 3...", EFI_YELLOW, EFI_BLACK);
    let _ = (bs.stall)(1_000_000);
    
    screen.put_str_at(5, 11, "Starting in 2...", EFI_YELLOW, EFI_BLACK);
    let _ = (bs.stall)(1_000_000);
    
    screen.put_str_at(5, 11, "Starting in 1...", EFI_YELLOW, EFI_BLACK);
    let _ = (bs.stall)(1_000_000);
}

/// Calibrate TSC frequency using UEFI Stall service.
/// Must be called BEFORE ExitBootServices.
fn calibrate_tsc_with_stall(bs: &crate::BootServices) -> u64 {
    // Read TSC before and after 10ms stall
    let start_tsc = read_tsc();
    
    // UEFI Stall takes microseconds - stall for 10ms (10,000 us)
    let _ = (bs.stall)(10_000);
    
    let end_tsc = read_tsc();
    
    // Calculate ticks for 10ms
    let ticks_10ms = end_tsc.saturating_sub(start_tsc);
    
    // Extrapolate to 1 second (multiply by 100)
    let tsc_freq = ticks_10ms.saturating_mul(100);
    
    // Sanity check: expect 1-10 GHz range
    if tsc_freq < 1_000_000_000 || tsc_freq > 10_000_000_000 {
        // Fallback to 2.5 GHz if result seems wrong
        2_500_000_000
    } else {
        tsc_freq
    }
}

/// Read TSC (Time Stamp Counter).
fn read_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack)
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// Probe for VirtIO NIC via PCI config space.
/// Returns (MMIO/IO base address, is_io_space) or (0, false) if not found.
fn probe_virtio_nic_with_debug(screen: &mut Screen, log_y: &mut usize) -> (u64, bool) {
    // VirtIO vendor ID: 0x1AF4
    // VirtIO-net device IDs: 0x1000 (transitional), 0x1041 (modern)
    const VIRTIO_VENDOR: u16 = 0x1AF4;
    const VIRTIO_NET_LEGACY: u16 = 0x1000;
    const VIRTIO_NET_MODERN: u16 = 0x1041;
    
    // Use legacy PCI config space access (port I/O)
    const PCI_CONFIG_ADDR: u16 = 0xCF8;
    const PCI_CONFIG_DATA: u16 = 0xCFC;
    
    fn pci_read32(bus: u8, device: u8, func: u8, offset: u8) -> u32 {
        let addr: u32 = (1 << 31) // Enable bit
            | ((bus as u32) << 16)
            | ((device as u32) << 11)
            | ((func as u32) << 8)
            | ((offset as u32) & 0xFC);
        
        unsafe {
            core::arch::asm!(
                "out dx, eax",
                in("dx") PCI_CONFIG_ADDR,
                in("eax") addr,
                options(nomem, nostack)
            );
            let value: u32;
            core::arch::asm!(
                "in eax, dx",
                in("dx") PCI_CONFIG_DATA,
                out("eax") value,
                options(nomem, nostack)
            );
            value
        }
    }
    
    screen.put_str_at(7, *log_y, "Scanning PCI bus 0...", EFI_DARKGRAY, EFI_BLACK);
    *log_y += 1;
    
    // Scan PCI bus 0 (QEMU puts virtio devices here)
    for device in 0..32u8 {
        let id = pci_read32(0, device, 0, 0);
        
        // Skip empty slots
        if id == 0xFFFFFFFF || id == 0 {
            continue;
        }
        
        let vendor = (id & 0xFFFF) as u16;
        let dev_id = ((id >> 16) & 0xFFFF) as u16;
        
        // Show what we find
        screen.put_str_at(9, *log_y, &alloc::format!(
            "PCI 0:{:02}:0 - {:04x}:{:04x}", device, vendor, dev_id
        ), EFI_DARKGRAY, EFI_BLACK);
        *log_y += 1;
        
        // Check for VirtIO network device
        if vendor == VIRTIO_VENDOR && (dev_id == VIRTIO_NET_LEGACY || dev_id == VIRTIO_NET_MODERN) {
            screen.put_str_at(9, *log_y, "  ^ VirtIO-net found!", EFI_LIGHTGREEN, EFI_BLACK);
            *log_y += 1;
            
            // Read BAR0 for base address
            let bar0 = pci_read32(0, device, 0, 0x10);
            
            screen.put_str_at(9, *log_y, &alloc::format!("  BAR0: {:#010x}", bar0), EFI_DARKGRAY, EFI_BLACK);
            *log_y += 1;
            
            // Check if I/O BAR (bit 0 = 1) or Memory BAR (bit 0 = 0)
            if bar0 & 1 == 1 {
                // I/O BAR - mask off type bit
                let io_base = (bar0 & 0xFFFFFFFC) as u64;
                screen.put_str_at(9, *log_y, &alloc::format!("  I/O base: {:#x}", io_base), EFI_CYAN, EFI_BLACK);
                *log_y += 1;
                return (io_base, true);
            } else {
                // Memory BAR - mask off type bits
                let mmio_base = (bar0 & 0xFFFFFFF0) as u64;
                
                // For 64-bit BAR, read upper 32 bits
                let final_base = if (bar0 >> 1) & 3 == 2 {
                    let bar1 = pci_read32(0, device, 0, 0x14);
                    mmio_base | ((bar1 as u64) << 32)
                } else {
                    mmio_base
                };
                
                screen.put_str_at(9, *log_y, &alloc::format!("  MMIO base: {:#x}", final_base), EFI_CYAN, EFI_BLACK);
                *log_y += 1;
                return (final_base, false);
            }
        }
    }
    
    screen.put_str_at(7, *log_y, "No VirtIO-net device found on bus 0", EFI_RED, EFI_BLACK);
    *log_y += 1;
    
    (0, false) // Not found
}

/// Leak a string so it becomes 'static.
/// Safe to use since we're about to exit boot services anyway.
fn leak_string(s: &str) -> &'static str {
    let boxed = alloc::boxed::Box::new(alloc::string::String::from(s));
    alloc::boxed::Box::leak(boxed).as_str()
}
