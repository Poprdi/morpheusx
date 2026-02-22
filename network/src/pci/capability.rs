//! PCI Capability chain walking and VirtIO capability parsing.
//!
//! Provides functions to discover and parse VirtIO PCI Modern capabilities.
//!
//! # Reference
//! - PCI Spec 3.0 §6.7 (Capability List)
//! - VirtIO Spec 1.2 §4.1.4 (PCI Device Discovery)

use super::config::{pci_cfg_read16, pci_cfg_read32, pci_cfg_read8, PciAddr};

// ═══════════════════════════════════════════════════════════════════════════
// ASM BINDINGS
// ═══════════════════════════════════════════════════════════════════════════

extern "win64" {
    /// Check if device has capability list.
    fn asm_pci_has_capabilities(bus: u8, device: u8, function: u8) -> u32;

    /// Get first capability pointer.
    fn asm_pci_get_cap_ptr(bus: u8, device: u8, function: u8) -> u32;

    /// Find capability by ID.
    fn asm_pci_find_cap(bus: u8, device: u8, function: u8, cap_id: u8) -> u32;

    /// Find VirtIO capability by cfg_type.
    fn asm_pci_find_virtio_cap(bus: u8, device: u8, function: u8, cfg_type: u8) -> u32;

    /// Parse VirtIO capability at offset.
    fn asm_virtio_pci_parse_cap(
        bus: u8,
        device: u8,
        function: u8,
        cap_offset: u8,
        out: *mut VirtioCapInfo,
    ) -> u32;

    /// Read BAR value.
    /// Returns: RAX = address, RDX = 1 if memory / 0 if IO
    fn asm_virtio_pci_read_bar(bus: u8, device: u8, function: u8, bar_idx: u8) -> u64;

    /// Probe all VirtIO caps and fill array.
    fn asm_virtio_pci_probe_caps(bus: u8, device: u8, function: u8, out: *mut VirtioCapInfo)
        -> u32;
}

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// PCI capability ID: Vendor-specific (used by VirtIO).
pub const PCI_CAP_ID_VNDR: u8 = 0x09;

/// VirtIO PCI capability type: Common configuration.
pub const VIRTIO_PCI_CAP_COMMON: u8 = 1;

/// VirtIO PCI capability type: Notification area.
pub const VIRTIO_PCI_CAP_NOTIFY: u8 = 2;

/// VirtIO PCI capability type: ISR status.
pub const VIRTIO_PCI_CAP_ISR: u8 = 3;

/// VirtIO PCI capability type: Device-specific configuration.
pub const VIRTIO_PCI_CAP_DEVICE: u8 = 4;

/// VirtIO PCI capability type: PCI config access alternative.
pub const VIRTIO_PCI_CAP_PCI_CFG: u8 = 5;

// ═══════════════════════════════════════════════════════════════════════════
// TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Parsed VirtIO PCI capability information.
///
/// Layout must match ASM expectations (24 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtioCapInfo {
    /// Capability type (1=common, 2=notify, 3=isr, 4=device, 5=pci_cfg).
    pub cfg_type: u8,
    /// BAR index (0-5).
    pub bar: u8,
    /// Padding.
    pub _pad: [u8; 2],
    /// Offset within BAR.
    pub offset: u32,
    /// Length of region.
    pub length: u32,
    /// Notify offset multiplier (only valid for notify cap).
    pub notify_off_multiplier: u32,
    /// PCI config space offset where this cap was found.
    pub cap_offset: u8,
    /// Padding.
    pub _pad2: [u8; 7],
}

const _: () = assert!(core::mem::size_of::<VirtioCapInfo>() == 24);

/// Collection of all VirtIO PCI capabilities for a device.
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtioPciCaps {
    /// Common configuration capability.
    pub common: Option<VirtioCapInfo>,
    /// Notification capability.
    pub notify: Option<VirtioCapInfo>,
    /// ISR status capability.
    pub isr: Option<VirtioCapInfo>,
    /// Device-specific configuration capability.
    pub device: Option<VirtioCapInfo>,
    /// PCI config access capability.
    pub pci_cfg: Option<VirtioCapInfo>,
    /// BAR base addresses (resolved).
    pub bar_addrs: [u64; 6],
    /// Bitmask of found capabilities.
    pub found_mask: u8,
}

impl VirtioPciCaps {
    /// Check if all required capabilities are present.
    pub fn has_required(&self) -> bool {
        // Common and notify are required for basic operation
        self.common.is_some() && self.notify.is_some()
    }

    /// Get the address for common config access.
    pub fn common_cfg_addr(&self) -> Option<u64> {
        self.common
            .map(|c| self.bar_addrs[c.bar as usize] + c.offset as u64)
    }

    /// Get the address for notification.
    pub fn notify_addr(&self) -> Option<u64> {
        self.notify
            .map(|n| self.bar_addrs[n.bar as usize] + n.offset as u64)
    }

    /// Get the notify offset multiplier.
    pub fn notify_multiplier(&self) -> u32 {
        self.notify.map(|n| n.notify_off_multiplier).unwrap_or(0)
    }

    /// Get the address for device-specific config.
    pub fn device_cfg_addr(&self) -> Option<u64> {
        self.device
            .map(|d| self.bar_addrs[d.bar as usize] + d.offset as u64)
    }

    /// Get the address for ISR status.
    pub fn isr_addr(&self) -> Option<u64> {
        self.isr
            .map(|i| self.bar_addrs[i.bar as usize] + i.offset as u64)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC API
// ═══════════════════════════════════════════════════════════════════════════

/// Check if a PCI device supports capability list.
pub fn has_capabilities(addr: PciAddr) -> bool {
    unsafe { asm_pci_has_capabilities(addr.bus, addr.device, addr.function) != 0 }
}

/// Get the first capability pointer for a device.
pub fn get_cap_ptr(addr: PciAddr) -> Option<u8> {
    let ptr = unsafe { asm_pci_get_cap_ptr(addr.bus, addr.device, addr.function) };
    if ptr != 0 && ptr < 256 {
        Some(ptr as u8)
    } else {
        None
    }
}

/// Find a capability by ID.
pub fn find_cap(addr: PciAddr, cap_id: u8) -> Option<u8> {
    let offset = unsafe { asm_pci_find_cap(addr.bus, addr.device, addr.function, cap_id) };
    if offset != 0 && offset < 256 {
        Some(offset as u8)
    } else {
        None
    }
}

/// Find a VirtIO capability by cfg_type.
pub fn find_virtio_cap(addr: PciAddr, cfg_type: u8) -> Option<u8> {
    let offset = unsafe { asm_pci_find_virtio_cap(addr.bus, addr.device, addr.function, cfg_type) };
    if offset != 0 && offset < 256 {
        Some(offset as u8)
    } else {
        None
    }
}

/// Parse a VirtIO capability at the given config space offset.
pub fn parse_virtio_cap(addr: PciAddr, cap_offset: u8) -> Option<VirtioCapInfo> {
    let mut info = VirtioCapInfo::default();
    let result = unsafe {
        asm_virtio_pci_parse_cap(addr.bus, addr.device, addr.function, cap_offset, &mut info)
    };
    if result != 0 {
        Some(info)
    } else {
        None
    }
}

/// Read a BAR base address (ASM version).
pub fn read_bar(addr: PciAddr, bar_idx: u8) -> u64 {
    unsafe { asm_virtio_pci_read_bar(addr.bus, addr.device, addr.function, bar_idx) }
}

/// Read a BAR base address in pure Rust (no ASM dependency).
///
/// Handles 32-bit and 64-bit memory BARs, I/O BARs, and type bit masking.
fn read_bar_rust(addr: PciAddr, bar_idx: u8) -> u64 {
    if bar_idx > 5 {
        return 0;
    }
    // BAR0 is at PCI config offset 0x10, each BAR is 4 bytes
    let bar_offset = 0x10u8 + bar_idx * 4;
    let bar_low = pci_cfg_read32(addr, bar_offset);

    if bar_low == 0 {
        return 0;
    }

    // Bit 0: 0 = memory, 1 = I/O
    if bar_low & 0x01 != 0 {
        // I/O BAR — mask type bits (low 2 bits)
        return (bar_low & 0xFFFF_FFFC) as u64;
    }

    // Memory BAR — bits 2:1 encode type (00=32-bit, 10=64-bit)
    let bar_type = (bar_low >> 1) & 0x03;
    let base_low = (bar_low & 0xFFFF_FFF0) as u64;

    if bar_type == 0x02 && bar_idx < 5 {
        // 64-bit BAR: high dword is in the next BAR slot
        let bar_high = pci_cfg_read32(addr, bar_offset + 4);
        return ((bar_high as u64) << 32) | base_low;
    }

    base_low
}

/// Probe all VirtIO capabilities for a device.
///
/// Pure Rust implementation using `walk_capabilities_rust()` and
/// `pci_cfg_read*` — bypasses broken ASM cap-chain walker entirely.
///
/// # VirtIO PCI capability layout (VirtIO spec §4.1.4)
/// ```text
/// offset+0:  cap_vndr  (u8)  = 0x09
/// offset+1:  cap_next  (u8)
/// offset+2:  cap_len   (u8)
/// offset+3:  cfg_type  (u8)  — 1=common, 2=notify, 3=isr, 4=device, 5=pci_cfg
/// offset+4:  bar       (u8)  — BAR index 0-5
/// offset+5:  id        (u8)
/// offset+6:  padding   (u16)
/// offset+8:  offset    (u32) — offset within BAR
/// offset+12: length    (u32) — region length
/// For notify (cfg_type=2) only:
/// offset+16: notify_off_multiplier (u32)
/// ```
pub fn probe_virtio_caps(addr: PciAddr) -> VirtioPciCaps {
    let mut caps = VirtioPciCaps::default();

    for (cap_offset, cap_id) in walk_capabilities_rust(addr) {
        if cap_id != PCI_CAP_ID_VNDR {
            continue;
        }

        let cfg_type = pci_cfg_read8(addr, cap_offset + 3);
        let bar = pci_cfg_read8(addr, cap_offset + 4);
        let bar_offset = pci_cfg_read32(addr, cap_offset + 8);
        let length = pci_cfg_read32(addr, cap_offset + 12);
        let notify_off_multiplier = if cfg_type == VIRTIO_PCI_CAP_NOTIFY {
            pci_cfg_read32(addr, cap_offset + 16)
        } else {
            0
        };

        let info = VirtioCapInfo {
            cfg_type,
            bar,
            _pad: [0; 2],
            offset: bar_offset,
            length,
            notify_off_multiplier,
            cap_offset,
            _pad2: [0; 7],
        };

        match cfg_type {
            VIRTIO_PCI_CAP_COMMON => {
                caps.common = Some(info);
                caps.found_mask |= 1 << 0;
            }
            VIRTIO_PCI_CAP_NOTIFY => {
                caps.notify = Some(info);
                caps.found_mask |= 1 << 1;
            }
            VIRTIO_PCI_CAP_ISR => {
                caps.isr = Some(info);
                caps.found_mask |= 1 << 2;
            }
            VIRTIO_PCI_CAP_DEVICE => {
                caps.device = Some(info);
                caps.found_mask |= 1 << 3;
            }
            VIRTIO_PCI_CAP_PCI_CFG => {
                caps.pci_cfg = Some(info);
                caps.found_mask |= 1 << 4;
            }
            _ => {}
        }
    }

    // Read all BAR addresses in pure Rust
    for i in 0..6u8 {
        caps.bar_addrs[i as usize] = read_bar_rust(addr, i);
    }

    caps
}

// ═══════════════════════════════════════════════════════════════════════════
// PURE RUST FALLBACK (for capability walking without ASM)
// ═══════════════════════════════════════════════════════════════════════════

/// Walk capability chain in pure Rust (fallback).
pub fn walk_capabilities_rust(addr: PciAddr) -> impl Iterator<Item = (u8, u8)> {
    WalkCaps::new(addr)
}

struct WalkCaps {
    addr: PciAddr,
    current: u8,
    count: u8,
}

impl WalkCaps {
    fn new(addr: PciAddr) -> Self {
        let status = pci_cfg_read16(addr, super::config::offset::STATUS);
        let has_caps = (status & super::config::status::CAP_LIST) != 0;

        let start = if has_caps {
            pci_cfg_read8(addr, super::config::offset::CAP_PTR) & 0xFC
        } else {
            0
        };

        Self {
            addr,
            current: start,
            count: 0,
        }
    }
}

impl Iterator for WalkCaps {
    type Item = (u8, u8); // (offset, cap_id)

    fn next(&mut self) -> Option<Self::Item> {
        if self.current == 0 || self.count > 48 {
            return None;
        }

        self.count += 1;

        let offset = self.current;
        let header = pci_cfg_read16(self.addr, offset);
        let cap_id = (header & 0xFF) as u8;
        let next = ((header >> 8) & 0xFC) as u8;

        self.current = next;

        Some((offset, cap_id))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DEBUG / SERIAL LOG HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Dump all capabilities to serial (uses crate's serial_println if available).
#[cfg(feature = "serial_debug")]
pub fn dump_capabilities(addr: PciAddr) {
    use crate::mainloop::serial::{serial_print, serial_print_hex, serial_println};

    serial_print("PCI ");
    serial_print_hex(addr.bus as u64);
    serial_print(":");
    serial_print_hex(addr.device as u64);
    serial_print(".");
    serial_print_hex(addr.function as u64);
    serial_println(" capabilities:");

    for (offset, cap_id) in walk_capabilities_rust(addr) {
        serial_print("  [");
        serial_print_hex(offset as u64);
        serial_print("] ID=");
        serial_print_hex(cap_id as u64);

        if cap_id == PCI_CAP_ID_VNDR {
            // VirtIO cap - read cfg_type
            let cfg_type = pci_cfg_read8(addr, offset + 3);
            serial_print(" VirtIO cfg_type=");
            serial_print_hex(cfg_type as u64);

            // Read bar
            let bar = pci_cfg_read8(addr, offset + 4);
            serial_print(" bar=");
            serial_print_hex(bar as u64);

            // Read offset
            let bar_offset = pci_cfg_read32(addr, offset + 8);
            serial_print(" off=");
            serial_print_hex(bar_offset as u64);

            // Read length
            let length = pci_cfg_read32(addr, offset + 12);
            serial_print(" len=");
            serial_print_hex(length as u64);
        }

        serial_println("");
    }
}
