//! Dynamic NIC probe and driver factory.
//!
//! Probes PCI bus and creates appropriate driver based on detected hardware.
//! This is the main entry point for automatic driver selection.
//!
//! # Supported Devices
//! - VirtIO-net (QEMU, cloud VMs)
//! - Intel e1000e family (ThinkPad T450s, T520, etc.)
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::boot::probe::{probe_network_device, ProbeResult};
//!
//! let result = unsafe { probe_network_device(&dma, tsc_freq)? };
//! match result {
//!     ProbeResult::VirtIO(driver) => { /* use driver */ }
//!     ProbeResult::Intel(driver) => { /* use driver */ }
//!     ProbeResult::None => { /* no NIC found */ }
//! }
//! ```

use crate::dma::DmaRegion;
use crate::driver::intel::{
    find_intel_nic, enable_device, validate_mmio_access, IntelNicInfo,
    E1000eConfig, E1000eDriver, E1000eError,
};
use crate::driver::virtio::{VirtioConfig, VirtioNetDriver, VirtioInitError};
use crate::pci::config::{pci_cfg_read16, pci_cfg_read32, offset, PciAddr};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// VirtIO vendor ID
const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
/// VirtIO-net device ID range start
const VIRTIO_NET_DEVICE_START: u16 = 0x1000;
/// VirtIO-net modern device ID
const VIRTIO_NET_MODERN: u16 = 0x1041;

/// Intel vendor ID
const INTEL_VENDOR_ID: u16 = 0x8086;

// ═══════════════════════════════════════════════════════════════════════════
// PROBE ERRORS
// ═══════════════════════════════════════════════════════════════════════════

/// Probe and initialization errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    /// No network device found
    NoDevice,
    /// VirtIO initialization failed
    VirtioInitFailed,
    /// Intel e1000e initialization failed
    IntelInitFailed,
    /// BAR mapping failed
    BarMappingFailed,
    /// Device not responding
    DeviceNotResponding,
}

impl From<VirtioInitError> for ProbeError {
    fn from(_: VirtioInitError) -> Self {
        ProbeError::VirtioInitFailed
    }
}

impl From<E1000eError> for ProbeError {
    fn from(_: E1000eError) -> Self {
        ProbeError::IntelInitFailed
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DETECTED DEVICE INFO
// ═══════════════════════════════════════════════════════════════════════════

/// Information about a detected network device.
#[derive(Debug, Clone, Copy)]
pub enum DetectedNic {
    /// VirtIO network device
    VirtIO {
        pci_addr: PciAddr,
        mmio_base: u64,
    },
    /// Intel e1000e network device
    Intel(IntelNicInfo),
}

/// Result of successful probe and initialization.
pub enum ProbeResult {
    /// VirtIO driver
    VirtIO(VirtioNetDriver),
    /// Intel e1000e driver
    Intel(E1000eDriver),
}

// ═══════════════════════════════════════════════════════════════════════════
// PCI SCANNING
// ═══════════════════════════════════════════════════════════════════════════

/// Scan PCI bus for supported network devices.
///
/// Returns the first supported NIC found, preferring Intel over VirtIO
/// (for real hardware priority).
pub fn scan_for_nic() -> Option<DetectedNic> {
    // First try to find Intel NIC (real hardware)
    if let Some(info) = find_intel_nic() {
        return Some(DetectedNic::Intel(info));
    }

    // Fall back to VirtIO (QEMU, VMs)
    if let Some((pci_addr, mmio_base)) = find_virtio_nic() {
        return Some(DetectedNic::VirtIO { pci_addr, mmio_base });
    }

    None
}

/// Scan for VirtIO network device.
fn find_virtio_nic() -> Option<(PciAddr, u64)> {
    for bus in 0..=255u8 {
        for device in 0..32u8 {
            for function in 0..8u8 {
                let addr = PciAddr::new(bus, device, function);

                let vendor_id = pci_cfg_read16(addr, offset::VENDOR_ID);
                if vendor_id == 0xFFFF {
                    if function == 0 {
                        break;
                    }
                    continue;
                }

                if vendor_id != VIRTIO_VENDOR_ID {
                    if function == 0 {
                        let header = pci_cfg_read16(addr, offset::HEADER_TYPE) & 0x80;
                        if header == 0 {
                            break;
                        }
                    }
                    continue;
                }

                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
                
                // Check for VirtIO-net (transitional or modern)
                if device_id != VIRTIO_NET_DEVICE_START && device_id != VIRTIO_NET_MODERN {
                    continue;
                }

                // Read BAR0
                let bar0 = pci_cfg_read32(addr, offset::BAR0);
                if bar0 & 0x01 != 0 {
                    // I/O BAR - skip (need MMIO)
                    continue;
                }

                let is_64bit = (bar0 & 0x06) == 0x04;
                let mmio_base = if is_64bit {
                    let bar1 = pci_cfg_read32(addr, offset::BAR1);
                    ((bar1 as u64) << 32) | ((bar0 & 0xFFFFFFF0) as u64)
                } else {
                    (bar0 & 0xFFFFFFF0) as u64
                };

                return Some((addr, mmio_base));
            }
        }
    }

    None
}

// ═══════════════════════════════════════════════════════════════════════════
// DRIVER CREATION
// ═══════════════════════════════════════════════════════════════════════════

/// Probe for network device and create appropriate driver.
///
/// # Arguments
/// - `dma`: Pre-allocated DMA region
/// - `tsc_freq`: Calibrated TSC frequency
///
/// # Safety
/// - DMA region must be properly allocated with correct bus addresses
/// - TSC frequency must be calibrated
pub unsafe fn probe_and_create_driver(
    dma: &DmaRegion,
    tsc_freq: u64,
) -> Result<ProbeResult, ProbeError> {
    let detected = scan_for_nic().ok_or(ProbeError::NoDevice)?;

    match detected {
        DetectedNic::Intel(info) => {
            // Enable device (bus mastering, memory space)
            enable_device(info.pci_addr);

            // Validate MMIO access
            if !validate_mmio_access(info.mmio_base) {
                return Err(ProbeError::DeviceNotResponding);
            }

            // Create driver config
            let config = E1000eConfig {
                dma_cpu_base: dma.cpu_base(),
                dma_bus_base: dma.bus_base(),
                rx_queue_size: 32,
                tx_queue_size: 32,
                buffer_size: 2048,
                tsc_freq,
            };

            // Create driver
            let driver = E1000eDriver::new(info.mmio_base, config)?;
            Ok(ProbeResult::Intel(driver))
        }

        DetectedNic::VirtIO { pci_addr, mmio_base } => {
            // Enable device
            let cmd = pci_cfg_read16(pci_addr, offset::COMMAND);
            crate::pci::config::pci_cfg_write16(pci_addr, offset::COMMAND, cmd | 0x06);

            // Create VirtIO config
            let config = VirtioConfig {
                dma_cpu_base: dma.cpu_base(),
                dma_bus_base: dma.bus_base(),
                dma_size: dma.size(),
                queue_size: 32,
                buffer_size: 2048,
            };

            // Create driver
            let driver = VirtioNetDriver::new(mmio_base, config)?;
            Ok(ProbeResult::VirtIO(driver))
        }
    }
}

/// Create Intel e1000e driver from known parameters.
///
/// Use this when MMIO base is already known (e.g., from BootHandoff).
///
/// # Safety
/// - `mmio_base` must be valid MMIO address
/// - DMA region must be properly allocated
pub unsafe fn create_intel_driver(
    mmio_base: u64,
    dma: &DmaRegion,
    tsc_freq: u64,
) -> Result<E1000eDriver, E1000eError> {
    let config = E1000eConfig {
        dma_cpu_base: dma.cpu_base(),
        dma_bus_base: dma.bus_base(),
        rx_queue_size: 32,
        tx_queue_size: 32,
        buffer_size: 2048,
        tsc_freq,
    };

    E1000eDriver::new(mmio_base, config)
}

/// Create VirtIO driver from known parameters.
///
/// Use this when MMIO base is already known (e.g., from BootHandoff).
///
/// # Safety
/// - `mmio_base` must be valid MMIO address
/// - DMA region must be properly allocated
pub unsafe fn create_virtio_driver(
    mmio_base: u64,
    dma: &DmaRegion,
) -> Result<VirtioNetDriver, VirtioInitError> {
    let config = VirtioConfig {
        dma_cpu_base: dma.cpu_base(),
        dma_bus_base: dma.bus_base(),
        dma_size: dma.size(),
        queue_size: 32,
        buffer_size: 2048,
    };

    VirtioNetDriver::new(mmio_base, config)
}

// ═══════════════════════════════════════════════════════════════════════════
// DEVICE TYPE DETECTION
// ═══════════════════════════════════════════════════════════════════════════

/// Detected NIC type for handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NicType {
    None = 0,
    VirtIO = 1,
    Intel = 2,
}

/// Detect what type of NIC is present without initializing.
///
/// Useful for populating BootHandoff before ExitBootServices.
pub fn detect_nic_type() -> (NicType, Option<u64>, Option<PciAddr>) {
    // Check for Intel first (real hardware priority)
    if let Some(info) = find_intel_nic() {
        return (NicType::Intel, Some(info.mmio_base), Some(info.pci_addr));
    }

    // Check for VirtIO
    if let Some((pci_addr, mmio_base)) = find_virtio_nic() {
        return (NicType::VirtIO, Some(mmio_base), Some(pci_addr));
    }

    (NicType::None, None, None)
}
