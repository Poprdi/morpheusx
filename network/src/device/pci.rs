//! PCI bus enumeration and device discovery.
//!
//! This module provides utilities for scanning the PCI bus to find VirtIO
//! and other network devices. Supports both:
//!
//! - **ECAM (Enhanced Configuration Access Mechanism)** - Modern PCIe
//! - **Legacy I/O ports** - Older PCI (0xCF8/0xCFC)
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::device::pci::{PciScanner, EcamAccess};
//!
//! // For QEMU with PCIe ECAM at 0xB000_0000
//! let ecam = unsafe { EcamAccess::new(0xB000_0000 as *mut u8) };
//! let scanner = PciScanner::new(ecam);
//!
//! for device in scanner.scan_bus(0) {
//!     if device.is_virtio_net() {
//!         // Found VirtIO network device!
//!     }
//! }
//! ```

extern crate alloc;

use alloc::vec::Vec;

/// PCI vendor ID for VirtIO devices.
pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;

/// VirtIO device ID range for transitional devices.
pub const VIRTIO_DEVICE_ID_BASE: u16 = 0x1000;

/// VirtIO device ID for modern network device.
pub const VIRTIO_NET_DEVICE_ID: u16 = 0x1041;

/// PCI configuration space size for a single function.
pub const PCI_CONFIG_SIZE: usize = 256;

/// PCIe extended configuration space size.
pub const PCIE_CONFIG_SIZE: usize = 4096;

/// PCI device/function identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceFunction {
    /// Bus number (0-255).
    pub bus: u8,
    /// Device number (0-31).
    pub device: u8,
    /// Function number (0-7).
    pub function: u8,
}

impl DeviceFunction {
    /// Create a new device/function identifier.
    pub const fn new(bus: u8, device: u8, function: u8) -> Self {
        Self {
            bus,
            device,
            function,
        }
    }

    /// Calculate ECAM offset for this device.
    pub const fn ecam_offset(&self) -> usize {
        ((self.bus as usize) << 20)
            | ((self.device as usize) << 15)
            | ((self.function as usize) << 12)
    }
}

/// PCI device information from configuration space.
#[derive(Debug, Clone, Copy)]
pub struct PciDeviceInfo {
    /// Device location.
    pub location: DeviceFunction,
    /// Vendor ID.
    pub vendor_id: u16,
    /// Device ID.
    pub device_id: u16,
    /// Class code.
    pub class: u8,
    /// Subclass code.
    pub subclass: u8,
    /// Programming interface.
    pub prog_if: u8,
    /// Revision ID.
    pub revision: u8,
    /// Header type (0 = standard, 1 = bridge, 2 = cardbus).
    pub header_type: u8,
    /// Multi-function device flag.
    pub multifunction: bool,
}

impl PciDeviceInfo {
    /// Check if this is a VirtIO device.
    pub fn is_virtio(&self) -> bool {
        self.vendor_id == VIRTIO_VENDOR_ID
    }

    /// Check if this is a VirtIO network device.
    pub fn is_virtio_net(&self) -> bool {
        self.is_virtio()
            && (self.device_id == VIRTIO_NET_DEVICE_ID
                || self.device_id == VIRTIO_DEVICE_ID_BASE + 1)
    }

    /// Check if this is a network device (class 0x02).
    pub fn is_network(&self) -> bool {
        self.class == 0x02
    }

    /// Get VirtIO device type (for transitional devices).
    pub fn virtio_device_type(&self) -> Option<u8> {
        if !self.is_virtio() {
            return None;
        }

        if self.device_id >= VIRTIO_DEVICE_ID_BASE && self.device_id <= VIRTIO_DEVICE_ID_BASE + 0x3F
        {
            // Transitional device: type = device_id - 0x1000
            Some((self.device_id - VIRTIO_DEVICE_ID_BASE) as u8)
        } else if self.device_id >= 0x1040 && self.device_id <= 0x107F {
            // Modern device: type = device_id - 0x1040
            Some((self.device_id - 0x1040) as u8)
        } else {
            None
        }
    }
}

/// Trait for PCI configuration space access.
pub trait ConfigAccess {
    /// Read a 32-bit value from configuration space.
    ///
    /// # Safety
    ///
    /// The offset must be valid and aligned to 4 bytes.
    unsafe fn read32(&self, device: DeviceFunction, offset: u8) -> u32;

    /// Write a 32-bit value to configuration space.
    ///
    /// # Safety
    ///
    /// The offset must be valid and aligned to 4 bytes.
    unsafe fn write32(&self, device: DeviceFunction, offset: u8, value: u32);

    /// Read a 16-bit value from configuration space.
    unsafe fn read16(&self, device: DeviceFunction, offset: u8) -> u16 {
        let val32 = unsafe { self.read32(device, offset & !0x3) };
        let shift = ((offset & 0x2) * 8) as usize;
        ((val32 >> shift) & 0xFFFF) as u16
    }

    /// Read an 8-bit value from configuration space.
    unsafe fn read8(&self, device: DeviceFunction, offset: u8) -> u8 {
        let val32 = unsafe { self.read32(device, offset & !0x3) };
        let shift = ((offset & 0x3) * 8) as usize;
        ((val32 >> shift) & 0xFF) as u8
    }
}

/// ECAM (PCIe Enhanced Configuration Access Mechanism) implementation.
pub struct EcamAccess {
    /// Base address of ECAM region.
    base: *mut u8,
}

impl EcamAccess {
    /// Create a new ECAM accessor.
    ///
    /// # Safety
    ///
    /// - `base` must be a valid pointer to the ECAM memory region.
    /// - The region must be at least 256MB (for full bus range).
    pub unsafe fn new(base: *mut u8) -> Self {
        Self { base }
    }

    /// Get the base address.
    pub fn base(&self) -> *mut u8 {
        self.base
    }
}

impl ConfigAccess for EcamAccess {
    unsafe fn read32(&self, device: DeviceFunction, offset: u8) -> u32 {
        let addr = self.base.add(device.ecam_offset() + (offset as usize));
        // SAFETY: Caller guarantees valid ECAM region
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    unsafe fn write32(&self, device: DeviceFunction, offset: u8, value: u32) {
        let addr = self.base.add(device.ecam_offset() + (offset as usize));
        // SAFETY: Caller guarantees valid ECAM region
        unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
    }
}

// SAFETY: ECAM access is thread-safe if properly synchronized externally
unsafe impl Send for EcamAccess {}
unsafe impl Sync for EcamAccess {}

// =============================================================================
// External assembly functions for PCI I/O (compiled from pci_io.S)
// Using standalone assembly avoids compiler optimization issues with inline asm
// =============================================================================
#[cfg(target_arch = "x86_64")]
extern "C" {
    /// Read 32-bit value from PCI configuration space.
    /// Args: bus, device, function, offset (must be 4-byte aligned)
    fn pci_config_read32(bus: u64, device: u64, function: u64, offset: u64) -> u32;

    /// Write 32-bit value to PCI configuration space.
    /// Args: bus, device, function, offset, value
    fn pci_config_write32(bus: u64, device: u64, function: u64, offset: u64, value: u32);

    /// Test if PCI I/O ports are accessible.
    /// Returns the value read back from 0xCF8 (should have bit 31 set).
    fn pci_io_test() -> u32;
}

/// Legacy I/O port PCI configuration access (0xCF8/0xCFC).
/// 
/// Uses standalone assembly (pci_io.S) for reliable I/O port access
/// without compiler optimization interference.
#[cfg(target_arch = "x86_64")]
pub struct LegacyIoAccess;

#[cfg(target_arch = "x86_64")]
impl LegacyIoAccess {
    /// Create a new legacy I/O accessor.
    pub fn new() -> Self {
        Self
    }

    /// Test if PCI I/O ports are working.
    /// Returns true if the enable bit (bit 31) is readable after write.
    pub fn test_io_ports() -> bool {
        let result = unsafe { pci_io_test() };
        (result & 0x8000_0000) != 0
    }
}

#[cfg(target_arch = "x86_64")]
impl Default for LegacyIoAccess {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_arch = "x86_64")]
impl ConfigAccess for LegacyIoAccess {
    unsafe fn read32(&self, device: DeviceFunction, offset: u8) -> u32 {
        // Use standalone assembly for reliable I/O port access
        // The external function handles all the port I/O atomically
        // without any compiler interference
        pci_config_read32(
            device.bus as u64,
            device.device as u64,
            device.function as u64,
            offset as u64,
        )
    }

    unsafe fn write32(&self, device: DeviceFunction, offset: u8, value: u32) {
        // Use standalone assembly for reliable I/O port access
        pci_config_write32(
            device.bus as u64,
            device.device as u64,
            device.function as u64,
            offset as u64,
            value,
        )
    }
}

/// PCI bus scanner.
pub struct PciScanner<A: ConfigAccess> {
    access: A,
}

impl<A: ConfigAccess> PciScanner<A> {
    /// Create a new PCI scanner with the given configuration access method.
    pub fn new(access: A) -> Self {
        Self { access }
    }

    /// Get a reference to the configuration access method.
    pub fn access(&self) -> &A {
        &self.access
    }

    /// Probe a single device/function.
    ///
    /// Returns `Some(info)` if a device is present, `None` otherwise.
    pub fn probe(&self, location: DeviceFunction) -> Option<PciDeviceInfo> {
        // Read vendor/device ID
        let id = unsafe { self.access.read32(location, 0x00) };
        let vendor_id = (id & 0xFFFF) as u16;
        let device_id = ((id >> 16) & 0xFFFF) as u16;

        // 0xFFFF = no device present
        if vendor_id == 0xFFFF {
            return None;
        }

        // Read class/revision
        let class_rev = unsafe { self.access.read32(location, 0x08) };
        let revision = (class_rev & 0xFF) as u8;
        let prog_if = ((class_rev >> 8) & 0xFF) as u8;
        let subclass = ((class_rev >> 16) & 0xFF) as u8;
        let class = ((class_rev >> 24) & 0xFF) as u8;

        // Read header type
        let header = unsafe { self.access.read32(location, 0x0C) };
        let header_type = ((header >> 16) & 0x7F) as u8;
        let multifunction = (header >> 16) & 0x80 != 0;

        Some(PciDeviceInfo {
            location,
            vendor_id,
            device_id,
            class,
            subclass,
            prog_if,
            revision,
            header_type,
            multifunction,
        })
    }

    /// Scan a single bus for devices.
    pub fn scan_bus(&self, bus: u8) -> Vec<PciDeviceInfo> {
        let mut devices = Vec::new();

        for device in 0..32 {
            // Check function 0 first
            let location = DeviceFunction::new(bus, device, 0);
            if let Some(info) = self.probe(location) {
                devices.push(info);

                // If multifunction, check other functions
                if info.multifunction {
                    for function in 1..8 {
                        let loc = DeviceFunction::new(bus, device, function);
                        if let Some(func_info) = self.probe(loc) {
                            devices.push(func_info);
                        }
                    }
                }
            }
        }

        devices
    }

    /// Scan all buses for devices (simple linear scan).
    pub fn scan_all(&self) -> Vec<PciDeviceInfo> {
        let mut devices = Vec::new();

        // Simple: scan buses 0-255
        // More sophisticated implementations would follow bridge topology
        for bus in 0..=255u8 {
            devices.extend(self.scan_bus(bus));
        }

        devices
    }

    /// Find all VirtIO network devices.
    pub fn find_virtio_net(&self) -> Vec<PciDeviceInfo> {
        self.scan_bus(0)
            .into_iter()
            .filter(|d| d.is_virtio_net())
            .collect()
    }

    /// Find all network devices (any vendor).
    pub fn find_network(&self) -> Vec<PciDeviceInfo> {
        self.scan_bus(0)
            .into_iter()
            .filter(|d| d.is_network())
            .collect()
    }
}

/// Common ECAM base addresses for different platforms.
pub mod ecam_bases {
    /// QEMU Q35 machine type (also used by OVMF).
    pub const QEMU_Q35: usize = 0xB000_0000;

    /// QEMU i440FX machine type (legacy).
    pub const QEMU_I440FX: usize = 0xE000_0000;

    /// Intel platform (typical).
    pub const INTEL_TYPICAL: usize = 0xE000_0000;
}

/// Diagnostic utilities for PCI debugging.
#[cfg(target_arch = "x86_64")]
pub mod diagnostics {
    use super::*;

    /// Test I/O port access using standalone assembly.
    /// Returns (cf8_readback, host_bridge_id).
    pub fn raw_io_test() -> (u32, u32) {
        // Use our external assembly test function
        let cf8_readback = unsafe { pci_io_test() };
        
        // Also read host bridge
        let legacy = LegacyIoAccess::new();
        let host_id = unsafe { legacy.read32(DeviceFunction::new(0, 0, 0), 0x00) };
        
        (cf8_readback, host_id)
    }

    /// Read the host bridge (bus 0, dev 0, func 0) vendor/device ID.
    /// This should ALWAYS return a valid device (the host bridge) if PCI works.
    /// Returns (vendor_id, device_id).
    pub fn read_host_bridge() -> (u16, u16) {
        let legacy = LegacyIoAccess::new();
        let df = DeviceFunction::new(0, 0, 0);
        let id = unsafe { legacy.read32(df, 0x00) };
        let vendor = (id & 0xFFFF) as u16;
        let device = ((id >> 16) & 0xFFFF) as u16;
        (vendor, device)
    }

    /// Scan specific device locations where QEMU typically places VirtIO.
    /// Returns raw vendor/device values (0xFFFF = no device).
    pub fn probe_common_virtio_locations() -> Vec<(DeviceFunction, u16, u16)> {
        let legacy = LegacyIoAccess::new();
        let mut results = Vec::new();

        // QEMU typically places VirtIO devices at these locations
        let locations = [
            DeviceFunction::new(0, 1, 0),
            DeviceFunction::new(0, 2, 0),
            DeviceFunction::new(0, 3, 0),
            DeviceFunction::new(0, 4, 0),
            DeviceFunction::new(0, 5, 0),
            DeviceFunction::new(0, 6, 0),
            DeviceFunction::new(0, 31, 0), // ISA bridge on i440FX
        ];

        for loc in locations {
            let id = unsafe { legacy.read32(loc, 0x00) };
            let vendor = (id & 0xFFFF) as u16;
            let device = ((id >> 16) & 0xFFFF) as u16;
            results.push((loc, vendor, device));
        }

        results
    }

    /// Full diagnostic: returns structured info about PCI state.
    pub struct PciDiagnostic {
        /// Whether 0xCF8 read-back matches what we wrote
        pub io_port_works: bool,
        /// Raw value read back from 0xCF8
        pub cf8_readback: u32,
        /// Host bridge vendor ID (should be 0x8086 for Intel/QEMU)
        pub host_bridge_vendor: u16,
        /// Host bridge device ID
        pub host_bridge_device: u16,
        /// All devices found on bus 0
        pub bus0_device_count: usize,
        /// Devices at common VirtIO locations
        pub virtio_locations: Vec<(DeviceFunction, u16, u16)>,
    }

    /// Run full PCI diagnostics.
    pub fn run_diagnostics() -> PciDiagnostic {
        let (cf8_readback, _data) = raw_io_test();

        // Check if readback has enable bit set (bit 31)
        let io_port_works = (cf8_readback & 0x8000_0000) != 0;

        let (host_vendor, host_device) = read_host_bridge();

        let legacy = LegacyIoAccess::new();
        let scanner = PciScanner::new(legacy);
        let bus0_devices = scanner.scan_bus(0);

        let virtio_locs = probe_common_virtio_locations();

        PciDiagnostic {
            io_port_works,
            cf8_readback,
            host_bridge_vendor: host_vendor,
            host_bridge_device: host_device,
            bus0_device_count: bus0_devices.len(),
            virtio_locations: virtio_locs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_function_ecam_offset() {
        let df = DeviceFunction::new(0, 0, 0);
        assert_eq!(df.ecam_offset(), 0);

        let df = DeviceFunction::new(0, 1, 0);
        assert_eq!(df.ecam_offset(), 1 << 15);

        let df = DeviceFunction::new(0, 0, 1);
        assert_eq!(df.ecam_offset(), 1 << 12);

        let df = DeviceFunction::new(1, 0, 0);
        assert_eq!(df.ecam_offset(), 1 << 20);

        // Bus 0, Device 3, Function 0 (typical QEMU virtio-net location)
        let df = DeviceFunction::new(0, 3, 0);
        assert_eq!(df.ecam_offset(), 3 << 15);
    }

    #[test]
    fn test_pci_device_info_virtio_detection() {
        let info = PciDeviceInfo {
            location: DeviceFunction::new(0, 3, 0),
            vendor_id: VIRTIO_VENDOR_ID,
            device_id: 0x1001, // Transitional net (type 1 = network)
            class: 0x02,
            subclass: 0x00,
            prog_if: 0x00,
            revision: 0x00,
            header_type: 0x00,
            multifunction: false,
        };

        assert!(info.is_virtio());
        assert!(info.is_virtio_net());
        assert!(info.is_network());
        assert_eq!(info.virtio_device_type(), Some(1)); // Network
    }

    #[test]
    fn test_virtio_device_type() {
        let mut info = PciDeviceInfo {
            location: DeviceFunction::new(0, 0, 0),
            vendor_id: VIRTIO_VENDOR_ID,
            device_id: 0x1001, // Net transitional
            class: 0x02,
            subclass: 0x00,
            prog_if: 0x00,
            revision: 0x00,
            header_type: 0x00,
            multifunction: false,
        };

        assert_eq!(info.virtio_device_type(), Some(1)); // Network

        info.device_id = 0x1002; // Block transitional
        assert_eq!(info.virtio_device_type(), Some(2)); // Block

        info.device_id = 0x1041; // Net modern
        assert_eq!(info.virtio_device_type(), Some(1)); // Network
    }

    #[test]
    fn test_non_virtio_device() {
        let info = PciDeviceInfo {
            location: DeviceFunction::new(0, 0, 0),
            vendor_id: 0x8086, // Intel
            device_id: 0x100E, // e1000
            class: 0x02,
            subclass: 0x00,
            prog_if: 0x00,
            revision: 0x00,
            header_type: 0x00,
            multifunction: false,
        };

        assert!(!info.is_virtio());
        assert!(!info.is_virtio_net());
        assert!(info.is_network());
        assert_eq!(info.virtio_device_type(), None);
    }

    #[test]
    fn test_ecam_bases() {
        assert_eq!(ecam_bases::QEMU_Q35, 0xB000_0000);
        assert_eq!(ecam_bases::QEMU_I440FX, 0xE000_0000);
    }

    // Mock ConfigAccess for testing scanner
    struct MockAccess {
        devices: Vec<(DeviceFunction, u32, u32, u32)>, // (location, id, class_rev, header)
    }

    impl MockAccess {
        fn new() -> Self {
            Self {
                devices: Vec::new(),
            }
        }

        fn add_device(&mut self, loc: DeviceFunction, vendor: u16, device: u16, class: u8) {
            let id = (vendor as u32) | ((device as u32) << 16);
            let class_rev = (class as u32) << 24;
            let header = 0u32; // Single function
            self.devices.push((loc, id, class_rev, header));
        }
    }

    impl ConfigAccess for MockAccess {
        unsafe fn read32(&self, device: DeviceFunction, offset: u8) -> u32 {
            for (loc, id, class_rev, header) in &self.devices {
                if *loc == device {
                    return match offset {
                        0x00 => *id,
                        0x08 => *class_rev,
                        0x0C => *header,
                        _ => 0,
                    };
                }
            }
            0xFFFF_FFFF // No device
        }

        unsafe fn write32(&self, _device: DeviceFunction, _offset: u8, _value: u32) {
            // No-op for mock
        }
    }

    #[test]
    fn test_scanner_empty_bus() {
        let access = MockAccess::new();
        let scanner = PciScanner::new(access);
        let devices = scanner.scan_bus(0);
        assert!(devices.is_empty());
    }

    #[test]
    fn test_scanner_finds_virtio() {
        let mut access = MockAccess::new();
        access.add_device(
            DeviceFunction::new(0, 3, 0),
            VIRTIO_VENDOR_ID,
            0x1001,
            0x02,
        );

        let scanner = PciScanner::new(access);
        let virtio = scanner.find_virtio_net();

        assert_eq!(virtio.len(), 1);
        assert_eq!(virtio[0].location.device, 3);
    }

    #[test]
    fn test_scanner_multiple_devices() {
        let mut access = MockAccess::new();
        access.add_device(
            DeviceFunction::new(0, 1, 0),
            0x8086,
            0x100E,
            0x02, // Intel e1000
        );
        access.add_device(
            DeviceFunction::new(0, 3, 0),
            VIRTIO_VENDOR_ID,
            0x1001,
            0x02, // VirtIO net
        );

        let scanner = PciScanner::new(access);

        let all_net = scanner.find_network();
        assert_eq!(all_net.len(), 2);

        // Re-scan for just virtio (need new scanner due to move)
        let mut access2 = MockAccess::new();
        access2.add_device(
            DeviceFunction::new(0, 1, 0),
            0x8086,
            0x100E,
            0x02,
        );
        access2.add_device(
            DeviceFunction::new(0, 3, 0),
            VIRTIO_VENDOR_ID,
            0x1001,
            0x02,
        );
        let scanner2 = PciScanner::new(access2);
        let virtio = scanner2.find_virtio_net();
        assert_eq!(virtio.len(), 1);
    }
}
