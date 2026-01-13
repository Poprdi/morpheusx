//! Dynamic block device probe and driver factory.
//!
//! Probes PCI bus and creates appropriate block driver based on detected hardware.
//! This is the main entry point for automatic block driver selection.
//!
//! # Supported Devices
//! - VirtIO-blk (QEMU, cloud VMs)
//! - AHCI SATA (Intel - ThinkPad T450s, etc.)
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::boot::block_probe::{probe_block_device, BlockProbeResult};
//!
//! let result = unsafe { probe_block_device(&blk_dma, tsc_freq)? };
//! match result {
//!     BlockProbeResult::VirtIO(driver) => { /* use driver */ }
//!     BlockProbeResult::Ahci(driver) => { /* use driver */ }
//! }
//! ```

use crate::device::{UnifiedBlockDevice, UnifiedBlockError};
use crate::driver::ahci::{AhciConfig, AhciDriver, AhciInitError, AHCI_DEVICE_IDS, INTEL_VENDOR_ID};
use crate::driver::virtio_blk::{VirtioBlkConfig, VirtioBlkDriver, VirtioBlkInitError};
use crate::pci::config::{pci_cfg_read16, pci_cfg_read32, pci_cfg_write16, offset, PciAddr};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// VirtIO vendor ID
const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
/// VirtIO-blk device ID (transitional)
const VIRTIO_BLK_DEVICE_LEGACY: u16 = 0x1001;
/// VirtIO-blk device ID (modern)
const VIRTIO_BLK_DEVICE_MODERN: u16 = 0x1042;

/// PCI Class code for SATA AHCI controller
const PCI_CLASS_SATA_AHCI: u32 = 0x0106;

// ═══════════════════════════════════════════════════════════════════════════
// PROBE ERRORS
// ═══════════════════════════════════════════════════════════════════════════

/// Probe and initialization errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockProbeError {
    /// No block device found
    NoDevice,
    /// VirtIO-blk initialization failed
    VirtioInitFailed,
    /// AHCI initialization failed
    AhciInitFailed,
    /// BAR mapping failed
    BarMappingFailed,
    /// Device not responding
    DeviceNotResponding,
}

impl From<VirtioBlkInitError> for BlockProbeError {
    fn from(_: VirtioBlkInitError) -> Self {
        BlockProbeError::VirtioInitFailed
    }
}

impl From<AhciInitError> for BlockProbeError {
    fn from(_: AhciInitError) -> Self {
        BlockProbeError::AhciInitFailed
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DETECTED DEVICE INFO
// ═══════════════════════════════════════════════════════════════════════════

/// Information about a detected block device.
#[derive(Debug, Clone, Copy)]
pub enum DetectedBlockDevice {
    /// VirtIO-blk device
    VirtIO {
        pci_addr: PciAddr,
        mmio_base: u64,
    },
    /// AHCI SATA controller
    Ahci(AhciInfo),
}

/// Information about detected AHCI controller.
#[derive(Debug, Clone, Copy)]
pub struct AhciInfo {
    /// PCI address
    pub pci_addr: PciAddr,
    /// ABAR (AHCI Base Address Register) from BAR5
    pub abar: u64,
    /// Device ID
    pub device_id: u16,
}

/// Result of successful probe and initialization.
pub enum BlockProbeResult {
    /// VirtIO-blk driver
    VirtIO(VirtioBlkDriver),
    /// AHCI SATA driver
    Ahci(AhciDriver),
}

// ═══════════════════════════════════════════════════════════════════════════
// PCI SCANNING
// ═══════════════════════════════════════════════════════════════════════════

/// Scan PCI bus for supported block devices.
///
/// Returns the first supported block device found, preferring AHCI over VirtIO
/// (for real hardware priority - matches network probe behavior).
pub fn scan_for_block_device() -> Option<DetectedBlockDevice> {
    // First try to find AHCI controller (real hardware)
    if let Some(info) = find_ahci_controller() {
        return Some(DetectedBlockDevice::Ahci(info));
    }

    // Fall back to VirtIO-blk (QEMU, VMs)
    if let Some((pci_addr, mmio_base)) = find_virtio_blk() {
        return Some(DetectedBlockDevice::VirtIO { pci_addr, mmio_base });
    }

    None
}

/// Scan for AHCI SATA controller.
pub fn find_ahci_controller() -> Option<AhciInfo> {
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

                // Check for Intel
                if vendor_id != INTEL_VENDOR_ID {
                    if function == 0 {
                        let header = pci_cfg_read16(addr, offset::HEADER_TYPE) & 0x80;
                        if header == 0 {
                            break;
                        }
                    }
                    continue;
                }

                // Check class code for SATA AHCI
                let class_code = pci_cfg_read32(addr, offset::CLASS_CODE);
                let class = (class_code >> 8) & 0xFFFF;
                if class != PCI_CLASS_SATA_AHCI {
                    continue;
                }

                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);

                // Verify it's a known AHCI device
                if !AHCI_DEVICE_IDS.contains(&device_id) {
                    continue;
                }

                // Read BAR5 (ABAR - AHCI Base Address Register)
                // AHCI uses BAR5 for MMIO
                let bar5 = pci_cfg_read32(addr, offset::BAR5);
                if bar5 == 0 || (bar5 & 0x01) != 0 {
                    // BAR5 not present or is I/O (shouldn't happen for AHCI)
                    continue;
                }

                let is_64bit = (bar5 & 0x06) == 0x04;
                let abar = if is_64bit {
                    let bar5_high = pci_cfg_read32(addr, offset::BAR5 + 4);
                    ((bar5_high as u64) << 32) | ((bar5 & 0xFFFFFFF0) as u64)
                } else {
                    (bar5 & 0xFFFFFFF0) as u64
                };

                return Some(AhciInfo {
                    pci_addr: addr,
                    abar,
                    device_id,
                });
            }
        }
    }

    None
}

/// Scan for VirtIO-blk device.
fn find_virtio_blk() -> Option<(PciAddr, u64)> {
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

                // Check for VirtIO-blk (transitional or modern)
                if device_id != VIRTIO_BLK_DEVICE_LEGACY && device_id != VIRTIO_BLK_DEVICE_MODERN {
                    continue;
                }

                // Read BAR0
                let bar0 = pci_cfg_read32(addr, offset::BAR0);
                if bar0 & 0x01 != 0 {
                    // I/O BAR - skip
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

/// Block DMA region configuration.
///
/// Must be properly allocated before calling probe functions.
pub struct BlockDmaConfig {
    /// TSC frequency for timeouts
    pub tsc_freq: u64,

    // For VirtIO-blk
    /// Descriptor table (CPU pointer)
    pub virtio_desc_cpu: *mut u8,
    /// Descriptor table (physical)
    pub virtio_desc_phys: u64,
    /// Available ring (CPU pointer)
    pub virtio_avail_cpu: *mut u8,
    /// Available ring (physical)
    pub virtio_avail_phys: u64,
    /// Used ring (CPU pointer)
    pub virtio_used_cpu: *mut u8,
    /// Used ring (physical)
    pub virtio_used_phys: u64,
    /// Headers area (CPU pointer)
    pub virtio_headers_cpu: *mut u8,
    /// Headers area (physical)
    pub virtio_headers_phys: u64,
    /// Status area (CPU pointer)
    pub virtio_status_cpu: *mut u8,
    /// Status area (physical)
    pub virtio_status_phys: u64,
    /// Notify address (for MMIO mode)
    pub virtio_notify_addr: u64,
    /// Queue size
    pub queue_size: u16,

    // For AHCI
    /// Command List (CPU pointer, 1K aligned)
    pub ahci_cmd_list_cpu: *mut u8,
    /// Command List (physical)
    pub ahci_cmd_list_phys: u64,
    /// FIS Receive buffer (CPU pointer, 256-byte aligned)
    pub ahci_fis_cpu: *mut u8,
    /// FIS Receive buffer (physical)
    pub ahci_fis_phys: u64,
    /// Command Tables (CPU pointer, 128-byte aligned, 8KB total)
    pub ahci_cmd_tables_cpu: *mut u8,
    /// Command Tables (physical)
    pub ahci_cmd_tables_phys: u64,
    /// IDENTIFY buffer (CPU pointer, 512 bytes)
    pub ahci_identify_cpu: *mut u8,
    /// IDENTIFY buffer (physical)
    pub ahci_identify_phys: u64,
}

/// Enable PCI device (bus mastering, memory space).
fn enable_pci_device(addr: PciAddr) {
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    // Set bus master (bit 2) and memory space (bit 1)
    pci_cfg_write16(addr, offset::COMMAND, cmd | 0x06);
}

/// Probe for block device and create appropriate driver.
///
/// # Safety
/// - DMA regions must be properly allocated with correct bus addresses
/// - TSC frequency must be calibrated
pub unsafe fn probe_and_create_block_driver(
    config: &BlockDmaConfig,
) -> Result<BlockProbeResult, BlockProbeError> {
    let detected = scan_for_block_device().ok_or(BlockProbeError::NoDevice)?;

    match detected {
        DetectedBlockDevice::Ahci(info) => {
            // Enable device
            enable_pci_device(info.pci_addr);

            // Create AHCI config
            let ahci_config = AhciConfig {
                tsc_freq: config.tsc_freq,
                cmd_list_cpu: config.ahci_cmd_list_cpu,
                cmd_list_phys: config.ahci_cmd_list_phys,
                fis_cpu: config.ahci_fis_cpu,
                fis_phys: config.ahci_fis_phys,
                cmd_tables_cpu: config.ahci_cmd_tables_cpu,
                cmd_tables_phys: config.ahci_cmd_tables_phys,
                identify_cpu: config.ahci_identify_cpu,
                identify_phys: config.ahci_identify_phys,
            };

            // Create driver
            let driver = AhciDriver::new(info.abar, ahci_config)?;
            Ok(BlockProbeResult::Ahci(driver))
        }

        DetectedBlockDevice::VirtIO { pci_addr, mmio_base } => {
            // Enable device
            enable_pci_device(pci_addr);

            // Create VirtIO-blk config
            let virtio_config = VirtioBlkConfig {
                queue_size: config.queue_size,
                desc_phys: config.virtio_desc_phys,
                avail_phys: config.virtio_avail_phys,
                used_phys: config.virtio_used_phys,
                headers_phys: config.virtio_headers_phys,
                status_phys: config.virtio_status_phys,
                headers_cpu: config.virtio_headers_cpu as u64,
                status_cpu: config.virtio_status_cpu as u64,
                notify_addr: config.virtio_notify_addr,
                transport_type: 0, // MMIO
            };

            // Create driver
            let driver = VirtioBlkDriver::new(mmio_base, virtio_config)?;
            Ok(BlockProbeResult::VirtIO(driver))
        }
    }
}

/// Probe and create unified block device.
///
/// This is the main entry point for block device access.
///
/// # Safety
/// - DMA regions must be properly allocated
/// - TSC frequency must be calibrated
pub unsafe fn probe_unified_block_device(
    config: &BlockDmaConfig,
) -> Result<UnifiedBlockDevice, UnifiedBlockError> {
    match probe_and_create_block_driver(config) {
        Ok(BlockProbeResult::VirtIO(driver)) => Ok(UnifiedBlockDevice::VirtIO(driver)),
        Ok(BlockProbeResult::Ahci(driver)) => Ok(UnifiedBlockDevice::Ahci(driver)),
        Err(BlockProbeError::NoDevice) => Err(UnifiedBlockError::NoDevice),
        Err(BlockProbeError::VirtioInitFailed) => Err(UnifiedBlockError::NoDevice),
        Err(BlockProbeError::AhciInitFailed) => Err(UnifiedBlockError::NoDevice),
        Err(_) => Err(UnifiedBlockError::NoDevice),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DEVICE TYPE DETECTION
// ═══════════════════════════════════════════════════════════════════════════

/// Detected block device type for handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockDeviceType {
    None = 0,
    VirtIO = 1,
    Ahci = 2,
}

/// Detect what type of block device is present without initializing.
///
/// Useful for populating BootHandoff before ExitBootServices.
pub fn detect_block_device_type() -> (BlockDeviceType, Option<u64>, Option<PciAddr>) {
    // Check for AHCI first (real hardware priority)
    if let Some(info) = find_ahci_controller() {
        return (BlockDeviceType::Ahci, Some(info.abar), Some(info.pci_addr));
    }

    // Check for VirtIO-blk
    if let Some((pci_addr, mmio_base)) = find_virtio_blk() {
        return (BlockDeviceType::VirtIO, Some(mmio_base), Some(pci_addr));
    }

    (BlockDeviceType::None, None, None)
}
