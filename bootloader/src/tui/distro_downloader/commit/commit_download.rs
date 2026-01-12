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
    display_commit_countdown, display_final_warnings, display_preparation_header,
};
use super::pci::{probe_virtio_blk_with_debug, probe_virtio_nic_with_debug};
use super::resources::{
    allocate_dma_region, allocate_stack, prepare_boot_handoff, DMA_SIZE, STACK_SIZE,
};
use super::uefi::{
    calibrate_tsc_with_stall, exit_boot_services_with_retry, find_esp_lba, leak_string,
};

use crate::tui::renderer::{EFI_BLACK, EFI_CYAN, EFI_LIGHTGREEN, EFI_RED, EFI_YELLOW};

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

    // Phase 1: Setup display
    let mut log_y = display_preparation_header(screen);

    // Phase 2: Allocate DMA region
    screen.put_str_at(5, log_y, "Allocating DMA region...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;

    let dma_region = match allocate_dma_region(bs, screen, &mut log_y) {
        Ok(addr) => addr,
        Err(_) => fatal_hang(),
    };
    log_y += 1;

    // Phase 3: Allocate stack
    screen.put_str_at(
        5,
        log_y,
        "Allocating bare-metal stack...",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 1;

    let (_, stack_top) = match allocate_stack(bs, screen, &mut log_y) {
        Ok(result) => result,
        Err(_) => fatal_hang(),
    };
    log_y += 1;

    // Phase 4: Calibrate TSC
    screen.put_str_at(5, log_y, "Calibrating TSC timing...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;

    let tsc_freq = calibrate_tsc_with_stall(bs);
    screen.put_str_at(
        7,
        log_y,
        &alloc::format!("TSC: {} Hz", tsc_freq),
        EFI_CYAN,
        EFI_BLACK,
    );
    log_y += 2;

    // Phase 5: Probe VirtIO NIC
    screen.put_str_at(
        5,
        log_y,
        "Probing VirtIO network device...",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 1;

    let nic_probe = probe_virtio_nic_with_debug(screen, &mut log_y);
    if nic_probe.mmio_base == 0 {
        screen.put_str_at(
            7,
            log_y,
            "Ensure QEMU has: -device virtio-net-pci,netdev=...",
            EFI_RED,
            EFI_BLACK,
        );
        fatal_hang();
    }
    log_y += 1;

    // Phase 6: Probe VirtIO block device
    screen.put_str_at(
        5,
        log_y,
        "Probing VirtIO block device...",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 1;

    let blk_probe = probe_virtio_blk_with_debug(screen, &mut log_y);
    display_block_device_status(screen, &mut log_y, &blk_probe);
    log_y += 1;

    // Phase 7: Find ESP partition
    screen.put_str_at(5, log_y, "Locating ESP partition...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;

    let esp_lba = find_esp_lba(bs, image_handle).unwrap_or(2048);
    screen.put_str_at(
        7,
        log_y,
        &alloc::format!("ESP Start LBA: {}", esp_lba),
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    log_y += 1;

    // Phase 8: Prepare boot handoff
    screen.put_str_at(5, log_y, "Preparing boot handoff...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;

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
        &mut log_y,
    ) {
        Ok(handoff) => handoff,
        Err(_) => fatal_hang(),
    };
    log_y += 1;

    // Phase 9: Leak URL for bare-metal use
    let url_copy = leak_string(&config.iso_url);

    // Phase 10: Final warnings and exit boot services
    display_final_warnings(screen, &mut log_y, bs);

    // CRITICAL: Exit boot services NOW (no more screen output!)
    if exit_boot_services_with_retry(bs, image_handle).is_err() {
        fatal_hang();
    }

    // ═══════════════════════════════════════════════════════════════════════
    // WE ARE NOW IN BARE-METAL MODE - No UEFI, serial only, custom drivers
    // ═══════════════════════════════════════════════════════════════════════

    let _result = enter_network_boot_url(handoff_ref, url_copy, esp_lba);

    // If we get here, download completed - must reset (can't return to UEFI)
    fatal_hang()
}

/// Display block device probe status.
fn display_block_device_status(
    screen: &mut Screen,
    log_y: &mut usize,
    blk_probe: &crate::boot::network_boot::BlkProbeResult,
) {
    let has_blk = blk_probe.device_type != 0;
    if has_blk {
        if blk_probe.transport_type == 1 {
            // PCI Modern
            screen.put_str_at(
                7,
                *log_y,
                &alloc::format!(
                    "VirtIO-blk (PCI Modern) common_cfg: {:#x}",
                    blk_probe.common_cfg
                ),
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
        } else {
            // Legacy MMIO
            screen.put_str_at(
                7,
                *log_y,
                &alloc::format!("VirtIO-blk found at {:#x}", blk_probe.mmio_base),
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
        }
    } else {
        screen.put_str_at(
            7,
            *log_y,
            "No VirtIO-blk found (ISO won't be saved to disk)",
            EFI_YELLOW,
            EFI_BLACK,
        );
    }
}

/// Fatal hang - loop forever.
fn fatal_hang() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
