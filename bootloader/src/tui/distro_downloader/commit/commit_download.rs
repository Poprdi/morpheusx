//! Download commit flow - main orchestration for UEFI to bare-metal transition.
//!
//! This module coordinates the critical ExitBootServices transition:
//! 1. User confirms download from catalog
//! 2. Allocate resources (DMA, stack, handoff)
//! 3. Calibrate timing
//! 4. Call ExitBootServices (POINT OF NO RETURN)
//! 5. Jump to bare_metal_main for actual download
//!
//! # Safety
//! After ExitBootServices: no UEFI services, no heap, serial-only output.

extern crate alloc;

use crate::boot::network_boot::enter_network_boot_url;
use crate::tui::renderer::Screen;

// Re-export configuration
pub use super::display::DownloadCommitConfig;

use super::display::{
    display_commit_countdown, display_download_start, display_error_and_halt, DebugLog, LOG_CYAN,
    LOG_GREEN, LOG_RED, LOG_YELLOW,
};
use super::pci::{probe_virtio_blk_with_debug, probe_virtio_nic_with_debug};
use super::resources::{
    allocate_dma_region, allocate_stack, prepare_boot_handoff, DMA_SIZE, STACK_SIZE,
};
use super::uefi::{
    calibrate_tsc_with_stall, exit_boot_services_with_retry, find_esp_lba, leak_string,
};

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

    // Phase 4: Probe VirtIO NIC
    debug_log.add("Probing VirtIO network device...", LOG_YELLOW);
    let nic_probe = probe_virtio_nic_with_debug(screen, &mut 0);
    if nic_probe.mmio_base == 0 {
        debug_log.add("  ERROR: No VirtIO NIC found!", LOG_RED);
        display_error_and_halt(
            screen,
            &debug_log,
            "No VirtIO NIC found. Ensure QEMU has: -device virtio-net-pci",
            bs,
        );
    }
    debug_log.add(
        &alloc::format!("  NIC MMIO: {:#x}", nic_probe.mmio_base),
        LOG_GREEN,
    );

    // Phase 5: Probe VirtIO block device
    debug_log.add("Probing VirtIO block device...", LOG_YELLOW);
    let blk_probe = probe_virtio_blk_with_debug(screen, &mut 0);
    if blk_probe.device_type != 0 {
        if blk_probe.transport_type == 1 {
            debug_log.add(
                &alloc::format!("  BLK (PCI Modern) common_cfg: {:#x}", blk_probe.common_cfg),
                LOG_GREEN,
            );
        } else {
            debug_log.add(
                &alloc::format!("  BLK MMIO: {:#x}", blk_probe.mmio_base),
                LOG_GREEN,
            );
        }
    } else {
        debug_log.add("  No VirtIO-blk found (ISO won't persist)", LOG_YELLOW);
    }

    // Phase 6: Find ESP partition
    debug_log.add("Locating ESP partition...", LOG_YELLOW);
    let esp_lba = find_esp_lba(bs, image_handle).unwrap_or(2048);
    debug_log.add(&alloc::format!("  ESP Start LBA: {}", esp_lba), LOG_GREEN);

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
