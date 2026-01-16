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

/// Read a BAR base address.
pub fn read_bar(addr: PciAddr, bar_idx: u8) -> u64 {
    unsafe { asm_virtio_pci_read_bar(addr.bus, addr.device, addr.function, bar_idx) }
}

/// Probe all VirtIO capabilities for a device.
///
/// This is the main entry point for VirtIO PCI device discovery.
pub fn probe_virtio_caps(addr: PciAddr) -> VirtioPciCaps {
    let mut caps = VirtioPciCaps::default();

    // Array to receive capability info (5 caps * 24 bytes each)
    let mut cap_array = [VirtioCapInfo::default(); 5];

    // Probe via ASM
    let found = unsafe {
        asm_virtio_pci_probe_caps(addr.bus, addr.device, addr.function, cap_array.as_mut_ptr())
    };

    caps.found_mask = found as u8;

    // Map results to struct
    if found & (1 << 0) != 0 {
        caps.common = Some(cap_array[0]);
    }
    if found & (1 << 1) != 0 {
        caps.notify = Some(cap_array[1]);
    }
    if found & (1 << 2) != 0 {
        caps.isr = Some(cap_array[2]);
    }
    if found & (1 << 3) != 0 {
        caps.device = Some(cap_array[3]);
    }
    if found & (1 << 4) != 0 {
        caps.pci_cfg = Some(cap_array[4]);
    }

    // Read all BAR addresses
    for i in 0..6 {
        caps.bar_addrs[i] = read_bar(addr, i as u8);
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
    use crate::mainloop::bare_metal::{serial_print, serial_print_hex, serial_println};

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
