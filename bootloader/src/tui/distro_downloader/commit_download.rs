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

use crate::boot::network_boot::{
    enter_network_boot_url, prepare_handoff_with_blk, BlkProbeResult, NicProbeResult,
};
use crate::tui::renderer::{
    Screen, EFI_BLACK, EFI_CYAN, EFI_DARKGRAY, EFI_LIGHTGREEN, EFI_RED, EFI_YELLOW,
};
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
    screen.put_str_at(
        5,
        2,
        "=== Preparing Download Environment ===",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
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
        loop {
            core::hint::spin_loop();
        }
    }

    // Zero the DMA region
    ptr::write_bytes(dma_region as *mut u8, 0, DMA_SIZE);

    screen.put_str_at(
        7,
        log_y,
        &alloc::format!("DMA base: {:#x}", dma_region),
        EFI_CYAN,
        EFI_BLACK,
    );
    log_y += 2;

    // Phase 2: Allocate stack for bare-metal mode
    screen.put_str_at(
        5,
        log_y,
        "Allocating bare-metal stack...",
        EFI_YELLOW,
        EFI_BLACK,
    );
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
        loop {
            core::hint::spin_loop();
        }
    }

    let stack_top = stack_region + STACK_SIZE as u64; // Stack grows down
    screen.put_str_at(
        7,
        log_y,
        &alloc::format!("Stack: {:#x}", stack_region),
        EFI_CYAN,
        EFI_BLACK,
    );
    log_y += 2;

    // Phase 3: Calibrate TSC using UEFI Stall
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

    // Phase 4: Probe VirtIO NIC via PCI (just finding it, not initializing)
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
        loop {
            core::hint::spin_loop();
        }
    }
    log_y += 1;

    // Phase 4b: Probe VirtIO block device for disk writes
    screen.put_str_at(
        5,
        log_y,
        "Probing VirtIO block device...",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 1;

    let blk_probe = probe_virtio_blk_with_debug(screen, &mut log_y);
    // Check device_type for device presence (mmio_base is 0 for PCI Modern)
    let has_blk = blk_probe.device_type != 0;
    if has_blk {
        if blk_probe.transport_type == 1 {
            // PCI Modern
            screen.put_str_at(
                7,
                log_y,
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
                log_y,
                &alloc::format!("VirtIO-blk found at {:#x}", blk_probe.mmio_base),
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
        }
    } else {
        screen.put_str_at(
            7,
            log_y,
            "No VirtIO-blk found (ISO won't be saved to disk)",
            EFI_YELLOW,
            EFI_BLACK,
        );
    }
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
        loop {
            core::hint::spin_loop();
        }
    }

    let handoff_ptr = handoff_page as *mut BootHandoff;

    let handoff = prepare_handoff_with_blk(
        &nic_probe,
        &blk_probe,
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
    screen.put_str_at(
        5,
        log_y,
        "=== EXITING BOOT SERVICES ===",
        EFI_RED,
        EFI_BLACK,
    );
    log_y += 1;
    screen.put_str_at(5, log_y, "After this point:", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    screen.put_str_at(7, log_y, "- Screen output will stop", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    screen.put_str_at(
        7,
        log_y,
        "- Progress via serial console only",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 1;
    screen.put_str_at(
        7,
        log_y,
        "- System will reboot when done",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 2;

    // Brief pause for user to see message
    for _ in 0..200_000_000u64 {
        core::hint::spin_loop();
    }

    screen.put_str_at(5, log_y, "Exiting boot services NOW...", EFI_RED, EFI_BLACK);
    log_y += 1;

    // ═══════════════════════════════════════════════════════════════════════
    // POINT OF NO RETURN - EXIT BOOT SERVICES
    // ═══════════════════════════════════════════════════════════════════════

    // Show progress since get_memory_map can be slow
    screen.put_str_at(7, log_y, "Reading memory map...", EFI_YELLOW, EFI_BLACK);

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

    screen.put_str_at(
        7,
        log_y,
        "Memory map obtained       ",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );

    if status != 0 {
        // Cannot display error properly at this point
        loop {
            core::hint::spin_loop();
        }
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
            loop {
                core::hint::spin_loop();
            }
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
    loop {
        core::hint::spin_loop();
    }
}

/// Display countdown before committing to download.
fn display_commit_countdown(
    screen: &mut Screen,
    config: &DownloadCommitConfig,
    bs: &crate::BootServices,
) {
    screen.clear();

    screen.put_str_at(
        5,
        2,
        "=== Download Confirmation ===",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );

    screen.put_str_at(5, 4, "About to download:", EFI_YELLOW, EFI_BLACK);
    screen.put_str_at(7, 5, &config.distro_name, EFI_CYAN, EFI_BLACK);
    screen.put_str_at(
        7,
        6,
        &alloc::format!("Size: {} MB", config.iso_size / (1024 * 1024)),
        EFI_CYAN,
        EFI_BLACK,
    );

    screen.put_str_at(
        5,
        8,
        "WARNING: This will exit UEFI boot services!",
        EFI_RED,
        EFI_BLACK,
    );
    screen.put_str_at(
        5,
        9,
        "The system cannot be interrupted during download.",
        EFI_RED,
        EFI_BLACK,
    );

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
/// Returns NicProbeResult with full transport information.
fn probe_virtio_nic_with_debug(screen: &mut Screen, log_y: &mut usize) -> NicProbeResult {
    // VirtIO vendor ID: 0x1AF4
    // VirtIO-net device IDs: 0x1000 (transitional), 0x1041 (modern)
    const VIRTIO_VENDOR: u16 = 0x1AF4;
    const VIRTIO_NET_LEGACY: u16 = 0x1000;
    const VIRTIO_NET_MODERN: u16 = 0x1041;

    // PCI config space constants
    const PCI_STATUS_REG: u8 = 0x06;
    const PCI_CAP_PTR: u8 = 0x34;
    const PCI_CAP_ID_VNDR: u8 = 0x09; // Vendor-specific (VirtIO uses this)

    // VirtIO PCI capability types
    const VIRTIO_PCI_CAP_COMMON: u8 = 1;
    const VIRTIO_PCI_CAP_NOTIFY: u8 = 2;
    const VIRTIO_PCI_CAP_ISR: u8 = 3;
    const VIRTIO_PCI_CAP_DEVICE: u8 = 4;

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

    fn pci_read16(bus: u8, device: u8, func: u8, offset: u8) -> u16 {
        let val32 = pci_read32(bus, device, func, offset & 0xFC);
        ((val32 >> ((offset & 2) * 8)) & 0xFFFF) as u16
    }

    fn pci_read8(bus: u8, device: u8, func: u8, offset: u8) -> u8 {
        let val32 = pci_read32(bus, device, func, offset & 0xFC);
        ((val32 >> ((offset & 3) * 8)) & 0xFF) as u8
    }

    /// Read BAR address, handling 32-bit and 64-bit BARs.
    fn read_bar(bus: u8, device: u8, func: u8, bar_index: u8) -> u64 {
        let bar_offset = 0x10 + bar_index * 4;
        let bar_val = pci_read32(bus, device, func, bar_offset);

        if bar_val & 1 == 0 {
            // Memory BAR
            let base = (bar_val & 0xFFFFFFF0) as u64;
            if (bar_val >> 1) & 3 == 2 {
                // 64-bit BAR - read upper 32 bits from next BAR
                let bar_hi = pci_read32(bus, device, func, bar_offset + 4);
                base | ((bar_hi as u64) << 32)
            } else {
                base
            }
        } else {
            // I/O BAR
            (bar_val & 0xFFFFFFFC) as u64
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
        screen.put_str_at(
            9,
            *log_y,
            &alloc::format!("PCI 0:{:02}:0 - {:04x}:{:04x}", device, vendor, dev_id),
            EFI_DARKGRAY,
            EFI_BLACK,
        );
        *log_y += 1;

        // Check for VirtIO network device
        if vendor == VIRTIO_VENDOR && (dev_id == VIRTIO_NET_LEGACY || dev_id == VIRTIO_NET_MODERN) {
            let is_modern = dev_id == VIRTIO_NET_MODERN;
            screen.put_str_at(
                9,
                *log_y,
                &alloc::format!(
                    "  ^ VirtIO-net found! ({})",
                    if is_modern {
                        "PCI Modern"
                    } else {
                        "PCI Legacy/Transitional"
                    }
                ),
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            *log_y += 1;

            // Read BAR0 for base address
            let bar0 = pci_read32(0, device, 0, 0x10);

            screen.put_str_at(
                9,
                *log_y,
                &alloc::format!("  BAR0: {:#010x}", bar0),
                EFI_DARKGRAY,
                EFI_BLACK,
            );
            *log_y += 1;

            // Check if device has capabilities (for PCI Modern)
            let status = pci_read16(0, device, 0, PCI_STATUS_REG);
            let has_caps = (status & 0x10) != 0;

            if has_caps {
                screen.put_str_at(9, *log_y, "  PCI Capabilities present", EFI_CYAN, EFI_BLACK);
                *log_y += 1;

                // Capability info storage
                let mut common_bar: u8 = 0;
                let mut common_offset: u32 = 0;
                let mut notify_bar: u8 = 0;
                let mut notify_offset: u32 = 0;
                let mut notify_off_multiplier: u32 = 0;
                let mut isr_bar: u8 = 0;
                let mut isr_offset: u32 = 0;
                let mut device_bar: u8 = 0;
                let mut device_offset: u32 = 0;

                let mut found_common = false;
                let mut found_notify = false;
                let mut found_isr = false;
                let mut found_device = false;

                // Walk capability chain to find VirtIO caps
                let mut cap_offset = pci_read8(0, device, 0, PCI_CAP_PTR) & 0xFC;

                while cap_offset != 0 && cap_offset < 0xFF {
                    let cap_id = pci_read8(0, device, 0, cap_offset);
                    let next = pci_read8(0, device, 0, cap_offset + 1);

                    if cap_id == PCI_CAP_ID_VNDR {
                        // VirtIO capability structure (per spec 4.1.4.3):
                        // +0: cap_vndr (0x09)
                        // +1: cap_next
                        // +2: cap_len
                        // +3: cfg_type
                        // +4: bar
                        // +5-7: padding
                        // +8: offset (4 bytes)
                        // +12: length (4 bytes)
                        // For notify: +16: notify_off_multiplier (4 bytes)

                        let cfg_type = pci_read8(0, device, 0, cap_offset + 3);
                        let bar = pci_read8(0, device, 0, cap_offset + 4);
                        let offset = pci_read32(0, device, 0, cap_offset + 8);

                        let cap_name = match cfg_type {
                            1 => "common_cfg",
                            2 => "notify_cfg",
                            3 => "isr_cfg",
                            4 => "device_cfg",
                            5 => "pci_cfg",
                            _ => "unknown",
                        };

                        screen.put_str_at(
                            9,
                            *log_y,
                            &alloc::format!(
                                "    Cap @{:#04x}: type={} bar={} off={:#x}",
                                cap_offset,
                                cap_name,
                                bar,
                                offset
                            ),
                            EFI_DARKGRAY,
                            EFI_BLACK,
                        );
                        *log_y += 1;

                        match cfg_type {
                            VIRTIO_PCI_CAP_COMMON => {
                                found_common = true;
                                common_bar = bar;
                                common_offset = offset;
                            }
                            VIRTIO_PCI_CAP_NOTIFY => {
                                found_notify = true;
                                notify_bar = bar;
                                notify_offset = offset;
                                // Read notify_off_multiplier (at offset +16 in capability)
                                notify_off_multiplier = pci_read32(0, device, 0, cap_offset + 16);
                                screen.put_str_at(
                                    9,
                                    *log_y,
                                    &alloc::format!(
                                        "      notify_off_multiplier: {}",
                                        notify_off_multiplier
                                    ),
                                    EFI_DARKGRAY,
                                    EFI_BLACK,
                                );
                                *log_y += 1;
                            }
                            VIRTIO_PCI_CAP_ISR => {
                                found_isr = true;
                                isr_bar = bar;
                                isr_offset = offset;
                            }
                            VIRTIO_PCI_CAP_DEVICE => {
                                found_device = true;
                                device_bar = bar;
                                device_offset = offset;
                            }
                            _ => {}
                        }
                    }

                    cap_offset = next & 0xFC;
                }

                // If we found all required PCI Modern caps, use them
                if found_common && found_notify {
                    screen.put_str_at(
                        9,
                        *log_y,
                        "  PCI Modern: All required caps found!",
                        EFI_LIGHTGREEN,
                        EFI_BLACK,
                    );
                    *log_y += 1;

                    // Read BAR bases for each capability
                    let common_base = read_bar(0, device, 0, common_bar);
                    let notify_base = read_bar(0, device, 0, notify_bar);
                    let isr_base = if found_isr {
                        read_bar(0, device, 0, isr_bar)
                    } else {
                        0
                    };
                    let device_base = if found_device {
                        read_bar(0, device, 0, device_bar)
                    } else {
                        0
                    };

                    let common_cfg_addr = common_base + common_offset as u64;
                    let notify_cfg_addr = notify_base + notify_offset as u64;
                    let isr_cfg_addr = isr_base + isr_offset as u64;
                    let device_cfg_addr = device_base + device_offset as u64;

                    screen.put_str_at(
                        9,
                        *log_y,
                        &alloc::format!("  common_cfg: {:#x}", common_cfg_addr),
                        EFI_CYAN,
                        EFI_BLACK,
                    );
                    *log_y += 1;
                    screen.put_str_at(
                        9,
                        *log_y,
                        &alloc::format!("  notify_cfg: {:#x}", notify_cfg_addr),
                        EFI_CYAN,
                        EFI_BLACK,
                    );
                    *log_y += 1;

                    // Return PCI Modern result
                    return NicProbeResult::pci_modern(
                        common_cfg_addr,
                        notify_cfg_addr,
                        isr_cfg_addr,
                        device_cfg_addr,
                        notify_off_multiplier,
                        0,      // bus
                        device, // device
                        0,      // function
                    );
                }
            }

            // Fallback: Check if I/O BAR (bit 0 = 1) or Memory BAR (bit 0 = 0)
            if bar0 & 1 == 1 {
                // I/O BAR - mask off type bit (Legacy device)
                let io_base = (bar0 & 0xFFFFFFFC) as u64;
                screen.put_str_at(
                    9,
                    *log_y,
                    &alloc::format!("  I/O base: {:#x} (Legacy)", io_base),
                    EFI_CYAN,
                    EFI_BLACK,
                );
                *log_y += 1;
                // PCI Legacy (I/O ports) - use transport type 2
                let mut result = NicProbeResult::mmio(io_base, 0, device, 0);
                result.transport_type = 2; // TRANSPORT_PCI_LEGACY
                return result;
            } else {
                // Memory BAR - mask off type bits (MMIO transport)
                let mmio_base = (bar0 & 0xFFFFFFF0) as u64;

                // For 64-bit BAR, read upper 32 bits
                let final_base = if (bar0 >> 1) & 3 == 2 {
                    let bar1 = pci_read32(0, device, 0, 0x14);
                    mmio_base | ((bar1 as u64) << 32)
                } else {
                    mmio_base
                };

                screen.put_str_at(
                    9,
                    *log_y,
                    &alloc::format!("  MMIO base: {:#x}", final_base),
                    EFI_CYAN,
                    EFI_BLACK,
                );
                *log_y += 1;
                return NicProbeResult::mmio(final_base, 0, device, 0);
            }
        }
    }

    screen.put_str_at(
        7,
        *log_y,
        "No VirtIO-net device found on bus 0",
        EFI_RED,
        EFI_BLACK,
    );
    *log_y += 1;

    NicProbeResult::zeroed() // Not found
}

/// Probe for VirtIO-blk device via PCI config space.
/// Returns BlkProbeResult with device information.
/// Probe for VirtIO-blk device via PCI config space.
/// Returns BlkProbeResult with device information.
fn probe_virtio_blk_with_debug(screen: &mut Screen, log_y: &mut usize) -> BlkProbeResult {
    // VirtIO vendor ID: 0x1AF4
    // VirtIO-blk device IDs: 0x1001 (transitional), 0x1042 (modern)
    const VIRTIO_VENDOR: u16 = 0x1AF4;
    const VIRTIO_BLK_LEGACY: u16 = 0x1001;
    const VIRTIO_BLK_MODERN: u16 = 0x1042;

    // PCI config space access constants
    const PCI_CONFIG_ADDR: u16 = 0xCF8;
    const PCI_CONFIG_DATA: u16 = 0xCFC;
    const PCI_STATUS_REG: u8 = 0x06;
    const PCI_CAP_PTR: u8 = 0x34;
    const PCI_CAP_ID_VNDR: u8 = 0x09;

    // VirtIO capability types
    const VIRTIO_PCI_CAP_COMMON: u8 = 1;
    const VIRTIO_PCI_CAP_NOTIFY: u8 = 2;
    const VIRTIO_PCI_CAP_ISR: u8 = 3;
    const VIRTIO_PCI_CAP_DEVICE: u8 = 4;

    fn pci_read32(bus: u8, device: u8, func: u8, offset: u8) -> u32 {
        let addr: u32 = (1 << 31)
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

    fn pci_read16(bus: u8, device: u8, func: u8, offset: u8) -> u16 {
        let dword = pci_read32(bus, device, func, offset & 0xFC);
        ((dword >> ((offset & 2) * 8)) & 0xFFFF) as u16
    }

    fn pci_read8(bus: u8, device: u8, func: u8, offset: u8) -> u8 {
        let dword = pci_read32(bus, device, func, offset & 0xFC);
        ((dword >> ((offset & 3) * 8)) & 0xFF) as u8
    }

    fn read_bar(bus: u8, device: u8, func: u8, bar_index: u8) -> u64 {
        let bar_offset = 0x10 + bar_index * 4;
        let bar_val = pci_read32(bus, device, func, bar_offset);

        if bar_val & 1 == 0 {
            let base = (bar_val & 0xFFFFFFF0) as u64;
            if (bar_val >> 1) & 3 == 2 {
                let bar_hi = pci_read32(bus, device, func, bar_offset + 4);
                base | ((bar_hi as u64) << 32)
            } else {
                base
            }
        } else {
            (bar_val & 0xFFFFFFFC) as u64
        }
    }

    // Scan PCI bus 0 for VirtIO-blk
    for device in 0..32u8 {
        let id = pci_read32(0, device, 0, 0);

        if id == 0xFFFFFFFF || id == 0 {
            continue;
        }

        let vendor = (id & 0xFFFF) as u16;
        let dev_id = ((id >> 16) & 0xFFFF) as u16;

        // Check for VirtIO block device
        if vendor == VIRTIO_VENDOR && (dev_id == VIRTIO_BLK_LEGACY || dev_id == VIRTIO_BLK_MODERN) {
            let is_modern = dev_id == VIRTIO_BLK_MODERN;

            screen.put_str_at(
                9,
                *log_y,
                &alloc::format!(
                    "PCI 0:{:02}:0 - VirtIO-blk ({})",
                    device,
                    if is_modern { "Modern" } else { "Legacy" }
                ),
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            *log_y += 1;

            // Check for PCI capabilities (required for Modern)
            let status = pci_read16(0, device, 0, PCI_STATUS_REG);
            let has_caps = (status & 0x10) != 0;

            if is_modern && has_caps {
                // PCI Modern: Parse capability chain to find common_cfg, notify, isr, device
                let mut common_bar: u8 = 0;
                let mut common_offset: u32 = 0;
                let mut notify_bar: u8 = 0;
                let mut notify_offset: u32 = 0;
                let mut notify_off_multiplier: u32 = 0;
                let mut isr_bar: u8 = 0;
                let mut isr_offset: u32 = 0;
                let mut device_bar: u8 = 0;
                let mut device_offset: u32 = 0;
                let mut found_common = false;
                let mut found_notify = false;
                let mut found_isr = false;
                let mut found_device = false;

                let mut cap_offset = pci_read8(0, device, 0, PCI_CAP_PTR) & 0xFC;

                while cap_offset != 0 && cap_offset < 0xFF {
                    let cap_id = pci_read8(0, device, 0, cap_offset);
                    let next = pci_read8(0, device, 0, cap_offset + 1);

                    if cap_id == PCI_CAP_ID_VNDR {
                        let cfg_type = pci_read8(0, device, 0, cap_offset + 3);
                        let bar = pci_read8(0, device, 0, cap_offset + 4);
                        let offset = pci_read32(0, device, 0, cap_offset + 8);

                        match cfg_type {
                            VIRTIO_PCI_CAP_COMMON => {
                                found_common = true;
                                common_bar = bar;
                                common_offset = offset;
                            }
                            VIRTIO_PCI_CAP_NOTIFY => {
                                found_notify = true;
                                notify_bar = bar;
                                notify_offset = offset;
                                notify_off_multiplier = pci_read32(0, device, 0, cap_offset + 16);
                            }
                            VIRTIO_PCI_CAP_ISR => {
                                found_isr = true;
                                isr_bar = bar;
                                isr_offset = offset;
                            }
                            VIRTIO_PCI_CAP_DEVICE => {
                                found_device = true;
                                device_bar = bar;
                                device_offset = offset;
                            }
                            _ => {}
                        }
                    }

                    cap_offset = next & 0xFC;
                }

                if found_common && found_notify {
                    let common_base = read_bar(0, device, 0, common_bar);
                    let notify_base = read_bar(0, device, 0, notify_bar);
                    let common_cfg_addr = common_base + common_offset as u64;
                    let notify_cfg_addr = notify_base + notify_offset as u64;

                    // Get ISR and device cfg addresses if found
                    let isr_cfg_addr = if found_isr {
                        read_bar(0, device, 0, isr_bar) + isr_offset as u64
                    } else {
                        0
                    };

                    let device_cfg_addr = if found_device {
                        read_bar(0, device, 0, device_bar) + device_offset as u64
                    } else {
                        0
                    };

                    screen.put_str_at(
                        9,
                        *log_y,
                        &alloc::format!("  common_cfg: {:#x}", common_cfg_addr),
                        EFI_CYAN,
                        EFI_BLACK,
                    );
                    *log_y += 1;
                    screen.put_str_at(
                        9,
                        *log_y,
                        &alloc::format!("  notify_cfg: {:#x}", notify_cfg_addr),
                        EFI_CYAN,
                        EFI_BLACK,
                    );
                    *log_y += 1;

                    // Return PCI Modern result
                    return BlkProbeResult::pci_modern(
                        common_cfg_addr,
                        notify_cfg_addr,
                        isr_cfg_addr,
                        device_cfg_addr,
                        notify_off_multiplier,
                        0,      // bus
                        device, // device
                        0,      // function
                    );
                }
            }

            // Fallback to Legacy BAR0
            let bar0 = pci_read32(0, device, 0, 0x10);
            let mmio_base = if bar0 & 1 == 0 {
                let base = (bar0 & 0xFFFFFFF0) as u64;
                if (bar0 >> 1) & 3 == 2 {
                    let bar1 = pci_read32(0, device, 0, 0x14);
                    base | ((bar1 as u64) << 32)
                } else {
                    base
                }
            } else {
                // I/O BAR - not supported for block
                screen.put_str_at(9, *log_y, "  I/O BAR not supported", EFI_RED, EFI_BLACK);
                *log_y += 1;
                continue;
            };

            screen.put_str_at(
                9,
                *log_y,
                &alloc::format!("  MMIO base: {:#x}", mmio_base),
                EFI_CYAN,
                EFI_BLACK,
            );
            *log_y += 1;

            return BlkProbeResult::virtio(mmio_base, 0, device, 0);
        }
    }

    BlkProbeResult::zeroed() // Not found
}

/// Leak a string so it becomes 'static.
/// Safe to use since we're about to exit boot services anyway.
fn leak_string(s: &str) -> &'static str {
    let boxed = alloc::boxed::Box::new(alloc::string::String::from(s));
    alloc::boxed::Box::leak(boxed).as_str()
}
