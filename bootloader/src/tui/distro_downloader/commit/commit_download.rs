//! Download commit flow - main orchestration for UEFI to bare-metal transition.
//!
//! This module coordinates the critical ExitBootServices transition:
//! 1. User confirms download from catalog
//! 2. Call ExitBootServices (POINT OF NO RETURN)
//! 3. hwinit takes ownership of the machine
//! 4. Download proceeds with drivers
//!
//! # NEW Architecture (Self-Contained)
//!
//! The new `commit_to_download_selfcontained` path is much simpler:
//! - Only saves memory map during ExitBootServices
//! - hwinit does ALL hardware init (GDT, IDT, PIC, heap, DMA, PCI)
//! - Drivers just do driver work
//!
//! # Safety
//! After ExitBootServices: no UEFI services, no heap, serial-only output.

extern crate alloc;

use crate::boot::gop::query_gop;
use crate::boot::network_boot::{
    // Legacy (old path)
    enter_network_boot_url,
    // New path - hwinit owns the world
    enter_baremetal_world, BaremetalEntryConfig, DownloadRequest,
};
use crate::tui::renderer::Screen;

// Re-export configuration
pub use super::display::DownloadCommitConfig;

use super::display::{
    display_commit_countdown, display_download_start, display_error_and_halt, DebugLog, LOG_CYAN,
    LOG_DARKGRAY, LOG_GREEN, LOG_RED, LOG_YELLOW,
};
use super::pci::{probe_ahci_with_debug, probe_nic_with_debug, probe_virtio_blk_with_debug};
use super::resources::{
    allocate_dma_region, allocate_stack, prepare_boot_handoff, DMA_SIZE, STACK_SIZE,
};
use super::uefi::{
    calibrate_tsc_with_stall, exit_boot_services_with_retry, find_esp_lba, leak_string,
};
use crate::boot::network_boot::{NIC_TYPE_INTEL, NIC_TYPE_VIRTIO};

use crate::tui::renderer::{EFI_BLACK, EFI_LIGHTGREEN, EFI_RED};

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

/// Commit to download - exits boot services and downloads in bare-metal mode.
///
/// # POINT OF NO RETURN
/// Once ExitBootServices is called, there's no going back.
///
/// # Safety
/// This function never returns on success. On failure, it loops forever.
pub unsafe fn commit_to_download(
    boot_services: *const crate::BootServices,
    image_handle: *mut (),
    screen: &mut Screen,
    config: DownloadCommitConfig,
) -> ! {
    let bs = &*boot_services;

    // Phase 0: Display countdown
    display_commit_countdown(screen, &config, bs);

    // Create debug log buffer (only displayed on error)
    let mut debug_log = DebugLog::new();

    // Phase 1: Allocate DMA region (silent, log for debug)
    debug_log.add("Allocating DMA region...", LOG_YELLOW);
    let dma_region = match allocate_dma_region(bs, screen, &mut 0) {
        Ok(addr) => {
            debug_log.add(&alloc::format!("  DMA region: {:#x}", addr), LOG_GREEN);
            addr
        }
        Err(_) => {
            display_error_and_halt(screen, &debug_log, "Failed to allocate DMA region", bs);
        }
    };

    // Phase 2: Allocate stack (silent)
    debug_log.add("Allocating bare-metal stack...", LOG_YELLOW);
    let (_, stack_top) = match allocate_stack(bs, screen, &mut 0) {
        Ok(result) => {
            debug_log.add(&alloc::format!("  Stack top: {:#x}", result.1), LOG_GREEN);
            result
        }
        Err(_) => {
            display_error_and_halt(screen, &debug_log, "Failed to allocate stack", bs);
        }
    };

    // Phase 3: Calibrate TSC
    debug_log.add("Calibrating TSC timing...", LOG_YELLOW);
    let tsc_freq = calibrate_tsc_with_stall(bs);
    debug_log.add(&alloc::format!("  TSC: {} Hz", tsc_freq), LOG_CYAN);

    // Phase 4: Probe network device (VirtIO or Intel e1000e)
    debug_log.add("Probing network device...", LOG_YELLOW);

    // First, dump PCI devices to debug log for diagnostics
    debug_log.add("  PCI bus scan:", LOG_CYAN);
    for bus in 0u8..16 {
        for device in 0u8..32 {
            let id = super::pci::config_space::pci_read32(bus, device, 0, 0);
            if id != 0xFFFFFFFF && id != 0 {
                let vendor = (id & 0xFFFF) as u16;
                let dev_id = ((id >> 16) & 0xFFFF) as u16;
                // Highlight Intel devices (vendor 0x8086) in cyan
                let color = if vendor == 0x8086 { LOG_CYAN } else { LOG_DARKGRAY };
                debug_log.add(
                    &alloc::format!(
                        "    {:02x}:{:02x}.0 = {:04x}:{:04x}",
                        bus,
                        device,
                        vendor,
                        dev_id
                    ),
                    color,
                );
            }
        }
    }

    let nic_probe = probe_nic_with_debug(screen, &mut 0);
    if nic_probe.mmio_base == 0 {
        debug_log.add("  ERROR: No supported NIC found!", LOG_RED);
        display_error_and_halt(
            screen,
            &debug_log,
            "No supported NIC found. Need VirtIO-net or Intel e1000e",
            bs,
        );
    }
    let nic_type_name = match nic_probe.nic_type {
        NIC_TYPE_VIRTIO => "VirtIO-net",
        NIC_TYPE_INTEL => "Intel e1000e",
        _ => "Unknown",
    };
    debug_log.add(
        &alloc::format!("  NIC: {} @ {:#x}", nic_type_name, nic_probe.mmio_base),
        LOG_GREEN,
    );

    // Phase 5: Probe block device (VirtIO first, then AHCI)
    debug_log.add("Probing block device...", LOG_YELLOW);
    let mut blk_probe = probe_virtio_blk_with_debug(screen, &mut 0);
    if blk_probe.device_type != 0 {
        if blk_probe.transport_type == 1 {
            debug_log.add(
                &alloc::format!(
                    "  BLK (VirtIO PCI Modern) common_cfg: {:#x}",
                    blk_probe.common_cfg
                ),
                LOG_GREEN,
            );
        } else {
            debug_log.add(
                &alloc::format!("  BLK (VirtIO) MMIO: {:#x}", blk_probe.mmio_base),
                LOG_GREEN,
            );
        }
    } else {
        // Try AHCI for real hardware
        debug_log.add("  No VirtIO-blk, probing AHCI...", LOG_YELLOW);
        blk_probe = probe_ahci_with_debug(screen, &mut 0);
        if blk_probe.device_type != 0 {
            debug_log.add(
                &alloc::format!("  BLK (AHCI) ABAR: {:#x}", blk_probe.mmio_base),
                LOG_GREEN,
            );
        } else {
            debug_log.add("  No block device found (ISO won't persist)", LOG_YELLOW);
        }
    }

    // Phase 6: Find ESP partition
    debug_log.add("Locating ESP partition...", LOG_YELLOW);
    let esp_lba = find_esp_lba(bs, image_handle).unwrap_or(2048);
    debug_log.add(&alloc::format!("  ESP Start LBA: {}", esp_lba), LOG_GREEN);

    // Phase 6.5: Query GOP for framebuffer info
    debug_log.add("Querying GOP framebuffer...", LOG_YELLOW);
    let gop_info = query_gop(bs);
    if let Some(ref fb) = gop_info {
        debug_log.add(
            &alloc::format!("  FB: {}x{} @ {:#x}", fb.width, fb.height, fb.base),
            LOG_GREEN,
        );
    } else {
        debug_log.add("  No GOP framebuffer available", LOG_YELLOW);
    }

    // Phase 7: Prepare boot handoff
    debug_log.add("Preparing boot handoff...", LOG_YELLOW);
    let handoff_ref = match prepare_boot_handoff(
        bs,
        &nic_probe,
        &blk_probe,
        dma_region,
        DMA_SIZE as u64,
        tsc_freq,
        stack_top,
        STACK_SIZE as u64,
        gop_info.as_ref(),
        screen,
        &mut 0,
    ) {
        Ok(handoff) => {
            debug_log.add("  Handoff prepared successfully", LOG_GREEN);
            handoff
        }
        Err(_) => {
            display_error_and_halt(screen, &debug_log, "Failed to prepare boot handoff", bs);
        }
    };

    // Phase 8: Leak URL for bare-metal use
    let url_copy = leak_string(&config.iso_url);
    debug_log.add("All systems ready!", LOG_GREEN);

    // SUCCESS: Show clean ASCII art (debug log not needed)
    display_download_start(screen, bs);

    // CRITICAL: Exit boot services NOW
    if exit_boot_services_with_retry(bs, image_handle).is_err() {
        // Can't show debug log after failed EBS attempt - just hang
        loop {
            core::hint::spin_loop();
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // WE ARE NOW IN BARE-METAL MODE - No UEFI, serial only, custom drivers
    // ═══════════════════════════════════════════════════════════════════════

    let _result = enter_network_boot_url(handoff_ref, url_copy, esp_lba);

    // If we get here, download completed - must reset (can't return to UEFI)
    loop {
        core::hint::spin_loop();
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// SELF-CONTAINED ARCHITECTURE (RECOMMENDED)
// ═══════════════════════════════════════════════════════════════════════════════

/// Commit to download using self-contained hwinit architecture.
///
/// This is the RECOMMENDED path. Much simpler than legacy:
/// - Allocates stack before EBS (only pre-EBS allocation needed)
/// - hwinit handles everything else
///
/// # POINT OF NO RETURN
/// Once ExitBootServices is called, there's no going back.
///
/// # Safety
/// This function never returns.
pub unsafe fn commit_to_download_selfcontained(
    boot_services: *const crate::BootServices,
    image_handle: *mut (),
    screen: &mut Screen,
    config: DownloadCommitConfig,
) -> ! {
    let bs = &*boot_services;

    // Phase 0: Display countdown
    display_commit_countdown(screen, &config, bs);

    // Create debug log buffer (only displayed on error)
    let mut debug_log = DebugLog::new();

    // ═══════════════════════════════════════════════════════════════════════
    // CRITICAL: Allocate stack BEFORE ExitBootServices
    // UEFI's stack may be in BootServicesData which becomes invalid after EBS
    // ═══════════════════════════════════════════════════════════════════════
    debug_log.add("Allocating bare-metal stack...", LOG_YELLOW);
    let (stack_base, stack_top) = match super::resources::allocate_stack(bs, screen, &mut 0) {
        Ok(result) => {
            debug_log.add(&alloc::format!("  Stack: {:#x} - {:#x}", result.0, result.1), LOG_GREEN);
            result
        }
        Err(_) => {
            display_error_and_halt(screen, &debug_log, "Failed to allocate stack", bs);
        }
    };

    // Phase 1: Find ESP partition (need this before EBS)
    debug_log.add("Locating ESP partition...", LOG_YELLOW);
    let esp_lba = super::uefi::find_esp_lba(bs, image_handle).unwrap_or(2048);
    debug_log.add(&alloc::format!("  ESP Start LBA: {}", esp_lba), LOG_GREEN);

    // Phase 2: Leak URL for bare-metal use (heap still available)
    let url_copy = leak_string(&config.iso_url);
    // Derive ISO name from distro name (or use a default)
    let name_copy = leak_string(&config.distro_name);

    debug_log.add("All systems ready!", LOG_GREEN);

    // SUCCESS: Show clean ASCII art
    display_download_start(screen, bs);

    // ═══════════════════════════════════════════════════════════════════════
    // CRITICAL: Exit boot services and capture memory map
    // ═══════════════════════════════════════════════════════════════════════

    // Use static buffers for data that needs to survive the stack switch
    // This is a one-shot operation, no concurrency concerns
    static mut MMAP_BUF: [u8; 8192] = [0u8; 8192];
    static mut MMAP_SIZE: usize = 0;
    static mut DESC_SIZE: usize = 0;
    static mut DESC_VERSION: u32 = 0;
    static mut URL_PTR: *const u8 = core::ptr::null();
    static mut URL_LEN: usize = 0;
    static mut NAME_PTR: *const u8 = core::ptr::null();
    static mut NAME_LEN: usize = 0;
    static mut ESP_LBA: u64 = 0;
    static mut NEW_STACK_TOP: u64 = 0;

    // Store values in statics before EBS
    URL_PTR = url_copy.as_ptr();
    URL_LEN = url_copy.len();
    NAME_PTR = name_copy.as_ptr();
    NAME_LEN = name_copy.len();
    ESP_LBA = esp_lba;
    NEW_STACK_TOP = stack_top;

    let mut map_key: usize = 0;
    MMAP_SIZE = MMAP_BUF.len();

    // Disable watchdog timer BEFORE GetMemoryMap
    let _ = (bs.set_watchdog_timer)(0, 0, 0, core::ptr::null());

    // Get memory map into static buffer
    let status = (bs.get_memory_map)(
        &mut MMAP_SIZE,
        MMAP_BUF.as_mut_ptr(),
        &mut map_key,
        &mut DESC_SIZE,
        &mut DESC_VERSION,
    );

    if status != 0 {
        loop { core::hint::spin_loop(); }
    }

    // Exit boot services IMMEDIATELY (memory map is stale if we do anything else)
    let status = (bs.exit_boot_services)(image_handle, map_key);
    if status != 0 {
        // Retry once with fresh map
        MMAP_SIZE = MMAP_BUF.len();
        let _ = (bs.get_memory_map)(
            &mut MMAP_SIZE,
            MMAP_BUF.as_mut_ptr(),
            &mut map_key,
            &mut DESC_SIZE,
            &mut DESC_VERSION,
        );
        let status = (bs.exit_boot_services)(image_handle, map_key);
        if status != 0 {
            loop { core::hint::spin_loop(); }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // UEFI IS GONE. Switch to our stack and enter hwinit.
    // ═══════════════════════════════════════════════════════════════════════

    // Switch to our allocated stack
    core::arch::asm!(
        "mov rsp, {new_stack}",
        "and rsp, ~0xF",  // 16-byte align
        new_stack = in(reg) NEW_STACK_TOP,
        options(nostack)
    );

    // Now on new stack - read from statics and call hwinit
    let entry_config = BaremetalEntryConfig {
        memory_map_ptr: MMAP_BUF.as_ptr(),
        memory_map_size: MMAP_SIZE,
        descriptor_size: DESC_SIZE,
        descriptor_version: DESC_VERSION,
    };

    let url_slice = core::str::from_utf8_unchecked(
        core::slice::from_raw_parts(URL_PTR, URL_LEN)
    );
    let name_slice = core::str::from_utf8_unchecked(
        core::slice::from_raw_parts(NAME_PTR, NAME_LEN)
    );

    let download_req = DownloadRequest {
        url: url_slice,
        name: name_slice,
        esp_start_lba: ESP_LBA,
    };

    enter_baremetal_world(entry_config, download_req);
}
