//! VirtIO block device PCI probing.

extern crate alloc;

use super::config_space::{pci_read16, pci_read32, pci_read8, read_bar};
use crate::boot::network_boot::BlkProbeResult;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_CYAN, EFI_LIGHTGREEN, EFI_RED, EFI_YELLOW};

/// VirtIO vendor and device IDs
const VIRTIO_VENDOR: u16 = 0x1AF4;
const VIRTIO_BLK_LEGACY: u16 = 0x1001;
const VIRTIO_BLK_MODERN: u16 = 0x1042;

/// PCI capability constants
const PCI_STATUS_REG: u8 = 0x06;
const PCI_CAP_PTR: u8 = 0x34;
const PCI_CAP_ID_VNDR: u8 = 0x09;

/// VirtIO capability types
const VIRTIO_PCI_CAP_COMMON: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY: u8 = 2;
const VIRTIO_PCI_CAP_ISR: u8 = 3;
const VIRTIO_PCI_CAP_DEVICE: u8 = 4;

/// Probe for VirtIO block device on PCI bus with debug output.
pub fn probe_virtio_blk_with_debug(screen: &mut Screen, log_y: &mut usize) -> BlkProbeResult {
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
            return probe_virtio_blk_device(screen, log_y, device, dev_id);
        }
    }

    BlkProbeResult::zeroed()
}

/// Probe a specific VirtIO block device.
fn probe_virtio_blk_device(
    screen: &mut Screen,
    log_y: &mut usize,
    device: u8,
    dev_id: u16,
) -> BlkProbeResult {
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

    // Check for PCI capabilities
    let status = pci_read16(0, device, 0, PCI_STATUS_REG);
    let has_caps = (status & 0x10) != 0;

    if is_modern && has_caps {
        if let Some(result) = try_pci_modern_caps(screen, log_y, device) {
            return result;
        }
    }

    // Fallback to Legacy BAR0
    probe_legacy_bar(screen, log_y, device)
}

/// Try to probe PCI Modern capabilities.
fn try_pci_modern_caps(
    screen: &mut Screen,
    log_y: &mut usize,
    device: u8,
) -> Option<BlkProbeResult> {
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

        return Some(BlkProbeResult::pci_modern(
            common_cfg_addr,
            notify_cfg_addr,
            isr_cfg_addr,
            device_cfg_addr,
            notify_off_multiplier,
            0,
            device,
            0,
        ));
    }

    None
}

/// Probe legacy BAR0.
fn probe_legacy_bar(screen: &mut Screen, log_y: &mut usize, device: u8) -> BlkProbeResult {
    let bar0 = pci_read32(0, device, 0, 0x10);

    if bar0 & 1 == 0 {
        let base = (bar0 & 0xFFFFFFF0) as u64;
        let mmio_base = if (bar0 >> 1) & 3 == 2 {
            let bar1 = pci_read32(0, device, 0, 0x14);
            base | ((bar1 as u64) << 32)
        } else {
            base
        };

        screen.put_str_at(
            9,
            *log_y,
            &alloc::format!("  MMIO base: {:#x}", mmio_base),
            EFI_CYAN,
            EFI_BLACK,
        );
        *log_y += 1;

        BlkProbeResult::virtio(mmio_base, 0, device, 0)
    } else {
        // I/O BAR not supported for block
        screen.put_str_at(9, *log_y, "  I/O BAR not supported", EFI_RED, EFI_BLACK);
        *log_y += 1;
        BlkProbeResult::zeroed()
    }
}
