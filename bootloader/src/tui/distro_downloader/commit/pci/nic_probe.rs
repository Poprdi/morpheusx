//! Network device PCI probing.
//!
//! Supports:
//! - VirtIO-net (QEMU, KVM)
//! - Intel e1000e (ThinkPad T450s, X240, T440s, etc.)

extern crate alloc;

use super::config_space::{pci_read16, pci_read32, pci_read8, read_bar};
use crate::boot::network_boot::{NicProbeResult, NIC_TYPE_INTEL, NIC_TYPE_VIRTIO};
use crate::tui::renderer::{
    Screen, EFI_BLACK, EFI_CYAN, EFI_DARKGRAY, EFI_LIGHTGREEN, EFI_RED, EFI_YELLOW,
};

/// VirtIO vendor and device IDs
const VIRTIO_VENDOR: u16 = 0x1AF4;
const VIRTIO_NET_LEGACY: u16 = 0x1000;
const VIRTIO_NET_MODERN: u16 = 0x1041;

/// Intel vendor ID
const INTEL_VENDOR: u16 = 0x8086;

/// Intel e1000e device IDs (common NICs found in ThinkPads and QEMU)
const INTEL_E1000E_DEVICES: &[u16] = &[
    0x100E, // 82540EM (QEMU e1000)
    0x10D3, // 82574L (QEMU e1000e)
    0x1502, // I218-LM (ThinkPad T450s, T440s, X240)
    0x1503, // I218-V
    0x10EA, // 82577LM
    0x10EB, // 82577LC
    0x10EF, // 82578DM
    0x10F0, // 82578DC
    0x1533, // I210
    0x1539, // I211
    0x156F, // I219-LM (Skylake+)
    0x1570, // I219-V (Skylake+)
    0x15B7, // I219-LM (Kaby Lake)
    0x15B8, // I219-V (Kaby Lake)
    0x15BB, // I219-LM (CNL)
    0x15BC, // I219-V (CNL)
    0x15BD, // I219-LM (CNL Corporate)
    0x15BE, // I219-V (CNL Corporate)
];

/// PCI capability constants
const PCI_STATUS_REG: u8 = 0x06;
const PCI_CAP_PTR: u8 = 0x34;
const PCI_CAP_ID_VNDR: u8 = 0x09;

/// VirtIO PCI capability types
const VIRTIO_PCI_CAP_COMMON: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY: u8 = 2;
const VIRTIO_PCI_CAP_ISR: u8 = 3;
const VIRTIO_PCI_CAP_DEVICE: u8 = 4;

/// Unified NIC probe - tries VirtIO first, then Intel e1000e.
///
/// This is the main entry point for NIC detection, supporting both
/// virtualized (VirtIO) and real hardware (Intel e1000e).
pub fn probe_nic_with_debug(screen: &mut Screen, log_y: &mut usize) -> NicProbeResult {
    screen.put_str_at(
        7,
        *log_y,
        "Scanning PCI for network devices...",
        EFI_DARKGRAY,
        EFI_BLACK,
    );
    *log_y += 1;

    // Scan all PCI buses (not just bus 0 - real hardware may be elsewhere)
    for bus in 0..=255u8 {
        for device in 0..32u8 {
            for function in 0..8u8 {
                let id = pci_read32(bus, device, function, 0);

                if id == 0xFFFFFFFF || id == 0 {
                    if function == 0 {
                        break; // No device at this slot
                    }
                    continue;
                }

                let vendor = (id & 0xFFFF) as u16;
                let dev_id = ((id >> 16) & 0xFFFF) as u16;

                // DEBUG: Show ALL PCI devices (helps diagnose detection issues)
                if bus < 2 || (vendor == INTEL_VENDOR && dev_id >= 0x1500 && dev_id <= 0x1600) {
                    screen.put_str_at(
                        7,
                        *log_y,
                        &alloc::format!(
                            "  PCI {:02x}:{:02x}.{} = {:04x}:{:04x}",
                            bus, device, function, vendor, dev_id
                        ),
                        EFI_DARKGRAY,
                        EFI_BLACK,
                    );
                    *log_y += 1;
                }

                // Check for VirtIO network device (highest priority)
                if vendor == VIRTIO_VENDOR
                    && (dev_id == VIRTIO_NET_LEGACY || dev_id == VIRTIO_NET_MODERN)
                {
                    screen.put_str_at(
                        9,
                        *log_y,
                        &alloc::format!(
                            "PCI {:02x}:{:02x}.{} - {:04x}:{:04x} VirtIO-net",
                            bus,
                            device,
                            function,
                            vendor,
                            dev_id
                        ),
                        EFI_LIGHTGREEN,
                        EFI_BLACK,
                    );
                    *log_y += 1;
                    return probe_virtio_nic_device(screen, log_y, bus, device, function, dev_id);
                }

                // Check for Intel e1000e
                if vendor == INTEL_VENDOR && INTEL_E1000E_DEVICES.contains(&dev_id) {
                    screen.put_str_at(
                        9,
                        *log_y,
                        &alloc::format!(
                            "PCI {:02x}:{:02x}.{} - {:04x}:{:04x} Intel e1000e",
                            bus,
                            device,
                            function,
                            vendor,
                            dev_id
                        ),
                        EFI_LIGHTGREEN,
                        EFI_BLACK,
                    );
                    *log_y += 1;
                    return probe_intel_nic_device(screen, log_y, bus, device, function, dev_id);
                }

                // Check for multi-function device
                if function == 0 {
                    let header = pci_read8(bus, device, function, 0x0E);
                    if header & 0x80 == 0 {
                        break; // Single-function device, skip other functions
                    }
                }
            }
        }
    }

    screen.put_str_at(7, *log_y, "No supported NIC found!", EFI_RED, EFI_BLACK);
    *log_y += 1;

    NicProbeResult::zeroed()
}

/// Probe for VirtIO NIC on PCI bus with debug output (legacy function for backwards compat).
pub fn probe_virtio_nic_with_debug(screen: &mut Screen, log_y: &mut usize) -> NicProbeResult {
    screen.put_str_at(
        7,
        *log_y,
        "Scanning PCI bus 0 for VirtIO...",
        EFI_DARKGRAY,
        EFI_BLACK,
    );
    *log_y += 1;

    // Scan PCI bus 0 (QEMU puts virtio devices here)
    for device in 0..32u8 {
        let id = pci_read32(0, device, 0, 0);

        if id == 0xFFFFFFFF || id == 0 {
            continue;
        }

        let vendor = (id & 0xFFFF) as u16;
        let dev_id = ((id >> 16) & 0xFFFF) as u16;

        // Check for VirtIO network device
        if vendor == VIRTIO_VENDOR && (dev_id == VIRTIO_NET_LEGACY || dev_id == VIRTIO_NET_MODERN) {
            return probe_virtio_nic_device(screen, log_y, 0, device, 0, dev_id);
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

    NicProbeResult::zeroed()
}

/// Probe a specific VirtIO NIC device.
fn probe_virtio_nic_device(
    screen: &mut Screen,
    log_y: &mut usize,
    bus: u8,
    device: u8,
    function: u8,
    dev_id: u16,
) -> NicProbeResult {
    let is_modern = dev_id == VIRTIO_NET_MODERN;
    screen.put_str_at(
        9,
        *log_y,
        &alloc::format!(
            "  VirtIO-net ({})",
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

    let bar0 = pci_read32(bus, device, function, 0x10);

    // Check for PCI capabilities
    let status = pci_read16(bus, device, function, PCI_STATUS_REG);
    let has_caps = (status & 0x10) != 0;

    if has_caps {
        if let Some(result) = try_pci_modern_caps(screen, log_y, bus, device, function) {
            return result;
        }
    }

    // Fallback to legacy BAR
    probe_virtio_legacy_bar(screen, log_y, bus, device, function, bar0)
}

/// Try to probe PCI Modern capabilities.
fn try_pci_modern_caps(
    screen: &mut Screen,
    log_y: &mut usize,
    bus: u8,
    device: u8,
    function: u8,
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
    let mut cap_offset = pci_read8(bus, device, function, PCI_CAP_PTR) & 0xFC;

    while cap_offset != 0 && cap_offset < 0xFF {
        let cap_id = pci_read8(bus, device, function, cap_offset);
        let next = pci_read8(bus, device, function, cap_offset + 1);

        if cap_id == PCI_CAP_ID_VNDR {
            let cfg_type = pci_read8(bus, device, function, cap_offset + 3);
            let bar = pci_read8(bus, device, function, cap_offset + 4);
            let offset = pci_read32(bus, device, function, cap_offset + 8);

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
                    notify_off_multiplier = pci_read32(bus, device, function, cap_offset + 16);
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

        let common_base = read_bar(bus, device, function, common_bar);
        let notify_base = read_bar(bus, device, function, notify_bar);
        let isr_base = if found_isr {
            read_bar(bus, device, function, isr_bar)
        } else {
            0
        };
        let device_base = if found_device {
            read_bar(bus, device, function, device_bar)
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

        return Some(NicProbeResult::pci_modern(
            common_cfg_addr,
            notify_cfg_addr,
            isr_cfg_addr,
            device_cfg_addr,
            notify_off_multiplier,
            bus,
            device,
            function,
        ));
    }

    None
}

/// Probe VirtIO legacy BAR (MMIO or I/O).
fn probe_virtio_legacy_bar(
    screen: &mut Screen,
    log_y: &mut usize,
    bus: u8,
    device: u8,
    function: u8,
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
        let mut result = NicProbeResult::virtio_mmio(io_base, bus, device, function);
        result.transport_type = 2; // TRANSPORT_PCI_LEGACY
        result
    } else {
        // Memory BAR - MMIO
        let mmio_base = (bar0 & 0xFFFFFFF0) as u64;
        let final_base = if (bar0 >> 1) & 3 == 2 {
            let bar1 = pci_read32(bus, device, function, 0x14);
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
        NicProbeResult::virtio_mmio(final_base, bus, device, function)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INTEL E1000E PROBING
// ═══════════════════════════════════════════════════════════════════════════

/// Probe an Intel e1000e NIC device.
fn probe_intel_nic_device(
    screen: &mut Screen,
    log_y: &mut usize,
    bus: u8,
    device: u8,
    function: u8,
    dev_id: u16,
) -> NicProbeResult {
    let name = match dev_id {
        0x100E => "82540EM (QEMU e1000)",
        0x10D3 => "82574L (QEMU e1000e)",
        0x1502 => "I218-LM (ThinkPad)",
        0x1503 => "I218-V",
        0x1533 => "I210",
        0x1539 => "I211",
        0x156F => "I219-LM",
        0x1570 => "I219-V",
        _ => "Intel e1000e",
    };

    screen.put_str_at(
        9,
        *log_y,
        &alloc::format!("  Intel {} (Device ID: {:#06x})", name, dev_id),
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    *log_y += 1;

    // Read BAR0 (MMIO)
    let bar0 = pci_read32(bus, device, function, 0x10);

    if bar0 & 1 != 0 {
        // I/O BAR - not supported for e1000e
        screen.put_str_at(
            9,
            *log_y,
            "  ERROR: I/O BAR not supported for Intel NIC",
            EFI_RED,
            EFI_BLACK,
        );
        *log_y += 1;
        return NicProbeResult::zeroed();
    }

    // Check if 64-bit BAR
    let is_64bit = (bar0 >> 1) & 0x3 == 2;
    let mmio_base = if is_64bit {
        let bar1 = pci_read32(bus, device, function, 0x14);
        ((bar0 & 0xFFFFFFF0) as u64) | ((bar1 as u64) << 32)
    } else {
        (bar0 & 0xFFFFFFF0) as u64
    };

    screen.put_str_at(
        9,
        *log_y,
        &alloc::format!("  MMIO base: {:#x}", mmio_base),
        EFI_CYAN,
        EFI_BLACK,
    );
    *log_y += 1;

    // Enable bus mastering and memory space access
    let cmd = pci_read16(bus, device, function, 0x04);
    let new_cmd = cmd | 0x06; // Memory Space + Bus Master
    pci_write16(bus, device, function, 0x04, new_cmd);

    screen.put_str_at(
        9,
        *log_y,
        &alloc::format!("  PCI Command: {:#06x} -> {:#06x}", cmd, new_cmd),
        EFI_DARKGRAY,
        EFI_BLACK,
    );
    *log_y += 1;

    NicProbeResult::intel(mmio_base, bus, device, function)
}

/// Write to PCI configuration space (16-bit).
fn pci_write16(bus: u8, device: u8, function: u8, offset: u8, value: u16) {
    use super::config_space::pci_write32;

    // Read-modify-write for 16-bit access
    let addr_offset = offset & 0xFC;
    let shift = ((offset & 2) * 8) as u32;

    let current = pci_read32(bus, device, function, addr_offset);
    let mask = !(0xFFFF_u32 << shift);
    let new_val = (current & mask) | ((value as u32) << shift);
    pci_write32(bus, device, function, addr_offset, new_val);
}
