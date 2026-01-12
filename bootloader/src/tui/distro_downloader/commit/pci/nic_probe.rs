//! VirtIO network device PCI probing.

extern crate alloc;

use super::config_space::{pci_read16, pci_read32, pci_read8, read_bar};
use crate::boot::network_boot::NicProbeResult;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_CYAN, EFI_DARKGRAY, EFI_LIGHTGREEN};

/// VirtIO vendor and device IDs
const VIRTIO_VENDOR: u16 = 0x1AF4;
const VIRTIO_NET_LEGACY: u16 = 0x1000;
const VIRTIO_NET_MODERN: u16 = 0x1041;

/// PCI capability constants
const PCI_STATUS_REG: u8 = 0x06;
const PCI_CAP_PTR: u8 = 0x34;
const PCI_CAP_ID_VNDR: u8 = 0x09;

/// VirtIO PCI capability types
const VIRTIO_PCI_CAP_COMMON: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY: u8 = 2;
const VIRTIO_PCI_CAP_ISR: u8 = 3;
const VIRTIO_PCI_CAP_DEVICE: u8 = 4;

/// Probe for VirtIO NIC on PCI bus with debug output.
pub fn probe_virtio_nic_with_debug(screen: &mut Screen, log_y: &mut usize) -> NicProbeResult {
    screen.put_str_at(7, *log_y, "Scanning PCI bus 0...", EFI_DARKGRAY, EFI_BLACK);
    *log_y += 1;

    // Scan PCI bus 0 (QEMU puts virtio devices here)
    for device in 0..32u8 {
        let id = pci_read32(0, device, 0, 0);

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
            return probe_virtio_nic_device(screen, log_y, device, dev_id);
        }
    }

    screen.put_str_at(
        7,
        *log_y,
        "No VirtIO-net device found on bus 0",
        crate::tui::renderer::EFI_RED,
        EFI_BLACK,
    );
    *log_y += 1;

    NicProbeResult::zeroed()
}

/// Probe a specific VirtIO NIC device.
fn probe_virtio_nic_device(
    screen: &mut Screen,
    log_y: &mut usize,
    device: u8,
    dev_id: u16,
) -> NicProbeResult {
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

    let bar0 = pci_read32(0, device, 0, 0x10);
    screen.put_str_at(
        9,
        *log_y,
        &alloc::format!("  BAR0: {:#010x}", bar0),
        EFI_DARKGRAY,
        EFI_BLACK,
    );
    *log_y += 1;

    // Check for PCI capabilities
    let status = pci_read16(0, device, 0, PCI_STATUS_REG);
    let has_caps = (status & 0x10) != 0;

    if has_caps {
        if let Some(result) = try_pci_modern_caps(screen, log_y, device) {
            return result;
        }
    }

    // Fallback to legacy BAR
    probe_legacy_bar(screen, log_y, device, bar0)
}

/// Try to probe PCI Modern capabilities.
fn try_pci_modern_caps(
    screen: &mut Screen,
    log_y: &mut usize,
    device: u8,
) -> Option<NicProbeResult> {
    screen.put_str_at(9, *log_y, "  PCI Capabilities present", EFI_CYAN, EFI_BLACK);
    *log_y += 1;

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

    // Walk capability chain
    let mut cap_offset = pci_read8(0, device, 0, PCI_CAP_PTR) & 0xFC;

    while cap_offset != 0 && cap_offset < 0xFF {
        let cap_id = pci_read8(0, device, 0, cap_offset);
        let next = pci_read8(0, device, 0, cap_offset + 1);

        if cap_id == PCI_CAP_ID_VNDR {
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
                    notify_off_multiplier = pci_read32(0, device, 0, cap_offset + 16);
                    screen.put_str_at(
                        9,
                        *log_y,
                        &alloc::format!("      notify_off_multiplier: {}", notify_off_multiplier),
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

    // If all required caps found, build modern result
    if found_common && found_notify {
        screen.put_str_at(
            9,
            *log_y,
            "  PCI Modern: All required caps found!",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        *log_y += 1;

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

        return Some(NicProbeResult::pci_modern(
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

/// Probe legacy BAR (MMIO or I/O).
fn probe_legacy_bar(
    screen: &mut Screen,
    log_y: &mut usize,
    device: u8,
    bar0: u32,
) -> NicProbeResult {
    if bar0 & 1 == 1 {
        // I/O BAR - Legacy device
        let io_base = (bar0 & 0xFFFFFFFC) as u64;
        screen.put_str_at(
            9,
            *log_y,
            &alloc::format!("  I/O base: {:#x} (Legacy)", io_base),
            EFI_CYAN,
            EFI_BLACK,
        );
        *log_y += 1;
        let mut result = NicProbeResult::mmio(io_base, 0, device, 0);
        result.transport_type = 2; // TRANSPORT_PCI_LEGACY
        result
    } else {
        // Memory BAR - MMIO
        let mmio_base = (bar0 & 0xFFFFFFF0) as u64;
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
        NicProbeResult::mmio(final_base, 0, device, 0)
    }
}
