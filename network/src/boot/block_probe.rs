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
use crate::driver::ahci::{AhciConfig, AhciDriver, AhciInitError, INTEL_VENDOR_ID};
use crate::driver::sdhci::{SdhciConfig, SdhciDriver, SdhciInitError};
use crate::driver::usb_msd::{UsbMsdConfig, UsbMsdDriver, UsbMsdInitError};
use crate::driver::virtio::transport::{PciModernConfig, VirtioTransport};
use crate::driver::virtio_blk::{VirtioBlkConfig, VirtioBlkDriver, VirtioBlkInitError};
use crate::pci::capability::probe_virtio_caps;
use crate::pci::config::{offset, pci_cfg_read16, pci_cfg_read32, pci_cfg_write16, PciAddr};

// ─── Inline serial helpers (network crate's serial_str + hex) ────────────
const VERBOSE_BLOCK_PROBE: bool = false;

fn dbg_str(s: &str) {
    if VERBOSE_BLOCK_PROBE {
        crate::serial_str(s);
    }
}
fn dbg_hex64(v: u64) {
    if !VERBOSE_BLOCK_PROBE {
        return;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    crate::serial_str("0x");
    for i in (0..16).rev() {
        crate::serial_byte(HEX[((v >> (i * 4)) & 0xF) as usize]);
    }
}
fn dbg_hex32(v: u32) {
    dbg_hex64(v as u64);
}
fn dbg_hex8(v: u8) {
    if !VERBOSE_BLOCK_PROBE {
        return;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    crate::serial_byte(HEX[(v >> 4) as usize]);
    crate::serial_byte(HEX[(v & 0xF) as usize]);
}

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// VirtIO vendor ID
const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
/// VirtIO-blk device ID (transitional)
const VIRTIO_BLK_DEVICE_LEGACY: u16 = 0x1001;
/// VirtIO-blk device ID (modern)
const VIRTIO_BLK_DEVICE_MODERN: u16 = 0x1042;

/// PCI subclass/prog-if for SATA AHCI controller (0x06/0x01).
///
/// We compare against `(class_code >> 8) & 0xFFFF`, which yields
/// subclass:prog_if (not class:subclass).
const PCI_CLASS_SATA_AHCI: u32 = 0x0601;
/// PCI class/subclass for SD Host Controller: 0x08/0x05.
///
/// Prog-if differs across controller revisions, so do not pin it to one value.
const PCI_CLASS_SUBCLASS_SDHCI: u32 = 0x0805;
/// PCI subclass/prog-if for USB xHCI: 0x03/0x30.
const PCI_CLASS_USB_XHCI: u32 = 0x0330;

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
    /// SDHCI initialization failed
    SdhciInitFailed,
    /// USB mass-storage initialization failed
    UsbMsdInitFailed,
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

impl From<SdhciInitError> for BlockProbeError {
    fn from(_: SdhciInitError) -> Self {
        BlockProbeError::SdhciInitFailed
    }
}

impl From<UsbMsdInitError> for BlockProbeError {
    fn from(_: UsbMsdInitError) -> Self {
        BlockProbeError::UsbMsdInitFailed
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DETECTED DEVICE INFO
// ═══════════════════════════════════════════════════════════════════════════

/// Information about a detected block device.
#[derive(Debug, Clone, Copy)]
pub enum DetectedBlockDevice {
    /// VirtIO-blk device
    VirtIO { pci_addr: PciAddr, mmio_base: u64 },
    /// AHCI SATA controller
    Ahci(AhciInfo),
    /// SDHCI controller
    Sdhci(SdhciInfo),
    /// USB xHCI controller candidate for USB MSD path
    UsbMsd(UsbMsdInfo),
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

/// Information about detected SDHCI controller.
#[derive(Debug, Clone, Copy)]
pub struct SdhciInfo {
    /// PCI address
    pub pci_addr: PciAddr,
    /// MMIO base from BAR0
    pub mmio_base: u64,
    /// Device ID
    pub device_id: u16,
}

/// Information about detected USB xHCI controller.
#[derive(Debug, Clone, Copy)]
pub struct UsbMsdInfo {
    /// PCI address
    pub pci_addr: PciAddr,
    /// MMIO base from BAR0
    pub mmio_base: u64,
    /// Device ID
    pub device_id: u16,
}

/// Result of successful probe and initialization.
pub enum BlockProbeResult {
    /// VirtIO-blk driver
    VirtIO(VirtioBlkDriver),
    /// AHCI SATA driver
    Ahci(AhciDriver),
    /// SDHCI block driver
    Sdhci(SdhciDriver),
    /// USB mass-storage block driver
    UsbMsd(UsbMsdDriver),
}

// ═══════════════════════════════════════════════════════════════════════════
// PCI SCANNING
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum block devices we can discover in a single scan.
const MAX_BLOCK_DEVICES: usize = 8;

/// Scan PCI bus for supported block devices.
///
/// Returns the first supported block device found, preferring AHCI over VirtIO
/// (for real hardware priority - matches network probe behavior).
pub fn scan_for_block_device() -> Option<DetectedBlockDevice> {
    // First try to find AHCI controller (real hardware)
    if let Some(info) = find_ahci_controller() {
        return Some(DetectedBlockDevice::Ahci(info));
    }

    // Then try SDHCI (SD card host)
    if let Some(info) = find_sdhci_controller() {
        return Some(DetectedBlockDevice::Sdhci(info));
    }

    // Then try USB xHCI for USB mass-storage path
    if let Some(info) = find_usb_xhci_controller() {
        return Some(DetectedBlockDevice::UsbMsd(info));
    }

    // Fall back to VirtIO-blk (QEMU, VMs)
    if let Some((pci_addr, mmio_base)) = find_virtio_blk() {
        return Some(DetectedBlockDevice::VirtIO {
            pci_addr,
            mmio_base,
        });
    }

    None
}

/// Scan PCI bus for ALL supported block devices.
///
/// Returns all detected AHCI and VirtIO-blk devices (up to 8).
/// AHCI devices are listed first, then VirtIO-blk.
pub fn scan_all_block_devices() -> ([Option<DetectedBlockDevice>; MAX_BLOCK_DEVICES], usize) {
    let mut result: [Option<DetectedBlockDevice>; MAX_BLOCK_DEVICES] = [None; MAX_BLOCK_DEVICES];
    let mut count = 0;

    // Collect all AHCI controllers.
    for bus in 0..=255u8 {
        if count >= MAX_BLOCK_DEVICES {
            break;
        }
        for device in 0..32u8 {
            if count >= MAX_BLOCK_DEVICES {
                break;
            }
            for function in 0..8u8 {
                if count >= MAX_BLOCK_DEVICES {
                    break;
                }
                let addr = PciAddr::new(bus, device, function);
                let vendor_id = pci_cfg_read16(addr, offset::VENDOR_ID);
                if vendor_id == 0xFFFF {
                    if function == 0 {
                        break;
                    }
                    continue;
                }
                let class_code = pci_cfg_read32(addr, offset::CLASS_CODE);
                let class = (class_code >> 8) & 0xFFFF;
                if class != PCI_CLASS_SATA_AHCI {
                    continue;
                }
                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
                let bar5 = pci_cfg_read32(addr, offset::BAR5);
                if bar5 == 0 || (bar5 & 0x01) != 0 {
                    continue;
                }
                let is_64bit = (bar5 & 0x06) == 0x04;
                let abar = if is_64bit {
                    let bar5_high = pci_cfg_read32(addr, offset::BAR5 + 4);
                    ((bar5_high as u64) << 32) | ((bar5 & 0xFFFFFFF0) as u64)
                } else {
                    (bar5 & 0xFFFFFFF0) as u64
                };
                result[count] = Some(DetectedBlockDevice::Ahci(AhciInfo {
                    pci_addr: addr,
                    abar,
                    device_id,
                }));
                count += 1;
            }
        }
    }

    // Collect all VirtIO-blk devices.
    // Collect all SDHCI controllers.
    for bus in 0..=255u8 {
        if count >= MAX_BLOCK_DEVICES {
            break;
        }
        for device in 0..32u8 {
            if count >= MAX_BLOCK_DEVICES {
                break;
            }
            for function in 0..8u8 {
                if count >= MAX_BLOCK_DEVICES {
                    break;
                }
                let addr = PciAddr::new(bus, device, function);
                let vendor_id = pci_cfg_read16(addr, offset::VENDOR_ID);
                if vendor_id == 0xFFFF {
                    if function == 0 {
                        break;
                    }
                    continue;
                }
                let class_code = pci_cfg_read32(addr, offset::CLASS_CODE);
                let class_subclass = (class_code >> 16) & 0xFFFF;
                if class_subclass != PCI_CLASS_SUBCLASS_SDHCI {
                    continue;
                }
                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
                let bar0 = pci_cfg_read32(addr, offset::BAR0);
                if bar0 == 0 || (bar0 & 0x01) != 0 {
                    continue;
                }
                let is_64bit = (bar0 & 0x06) == 0x04;
                let mmio_base = if is_64bit {
                    let bar1 = pci_cfg_read32(addr, offset::BAR1);
                    ((bar1 as u64) << 32) | ((bar0 & 0xFFFFFFF0) as u64)
                } else {
                    (bar0 & 0xFFFFFFF0) as u64
                };
                result[count] = Some(DetectedBlockDevice::Sdhci(SdhciInfo {
                    pci_addr: addr,
                    mmio_base,
                    device_id,
                }));
                count += 1;
            }
        }
    }

    // Collect all USB xHCI controllers.
    for bus in 0..=255u8 {
        if count >= MAX_BLOCK_DEVICES {
            break;
        }
        for device in 0..32u8 {
            if count >= MAX_BLOCK_DEVICES {
                break;
            }
            for function in 0..8u8 {
                if count >= MAX_BLOCK_DEVICES {
                    break;
                }
                let addr = PciAddr::new(bus, device, function);
                let vendor_id = pci_cfg_read16(addr, offset::VENDOR_ID);
                if vendor_id == 0xFFFF {
                    if function == 0 {
                        break;
                    }
                    continue;
                }
                let class_code = pci_cfg_read32(addr, offset::CLASS_CODE);
                let class = (class_code >> 8) & 0xFFFF;
                if class != PCI_CLASS_USB_XHCI {
                    continue;
                }
                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
                let bar0 = pci_cfg_read32(addr, offset::BAR0);
                if bar0 == 0 || (bar0 & 0x01) != 0 {
                    continue;
                }
                let is_64bit = (bar0 & 0x06) == 0x04;
                let mmio_base = if is_64bit {
                    let bar1 = pci_cfg_read32(addr, offset::BAR1);
                    ((bar1 as u64) << 32) | ((bar0 & 0xFFFFFFF0) as u64)
                } else {
                    (bar0 & 0xFFFFFFF0) as u64
                };
                result[count] = Some(DetectedBlockDevice::UsbMsd(UsbMsdInfo {
                    pci_addr: addr,
                    mmio_base,
                    device_id,
                }));
                count += 1;
            }
        }
    }

    // Collect all VirtIO-blk devices.
    for bus in 0..=255u8 {
        if count >= MAX_BLOCK_DEVICES {
            break;
        }
        for device in 0..32u8 {
            if count >= MAX_BLOCK_DEVICES {
                break;
            }
            for function in 0..8u8 {
                if count >= MAX_BLOCK_DEVICES {
                    break;
                }
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
                if device_id != VIRTIO_BLK_DEVICE_LEGACY && device_id != VIRTIO_BLK_DEVICE_MODERN {
                    continue;
                }
                let bar0 = pci_cfg_read32(addr, offset::BAR0);
                if bar0 & 0x01 != 0 {
                    continue;
                }
                let is_64bit = (bar0 & 0x06) == 0x04;
                let mmio_base = if is_64bit {
                    let bar1 = pci_cfg_read32(addr, offset::BAR1);
                    ((bar1 as u64) << 32) | ((bar0 & 0xFFFFFFF0) as u64)
                } else {
                    (bar0 & 0xFFFFFFF0) as u64
                };
                result[count] = Some(DetectedBlockDevice::VirtIO {
                    pci_addr: addr,
                    mmio_base,
                });
                count += 1;
            }
        }
    }

    (result, count)
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

                // Check class code for SATA AHCI
                let class_code = pci_cfg_read32(addr, offset::CLASS_CODE);
                let class = (class_code >> 8) & 0xFFFF;
                if class != PCI_CLASS_SATA_AHCI {
                    continue;
                }

                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);

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

/// Scan for SDHCI controller.
pub fn find_sdhci_controller() -> Option<SdhciInfo> {
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

                let class_code = pci_cfg_read32(addr, offset::CLASS_CODE);
                let class_subclass = (class_code >> 16) & 0xFFFF;
                if class_subclass != PCI_CLASS_SUBCLASS_SDHCI {
                    continue;
                }

                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
                let bar0 = pci_cfg_read32(addr, offset::BAR0);
                if bar0 == 0 || (bar0 & 0x01) != 0 {
                    continue;
                }

                let is_64bit = (bar0 & 0x06) == 0x04;
                let mmio_base = if is_64bit {
                    let bar1 = pci_cfg_read32(addr, offset::BAR1);
                    ((bar1 as u64) << 32) | ((bar0 & 0xFFFFFFF0) as u64)
                } else {
                    (bar0 & 0xFFFFFFF0) as u64
                };

                return Some(SdhciInfo {
                    pci_addr: addr,
                    mmio_base,
                    device_id,
                });
            }
        }
    }

    None
}

/// Scan for USB xHCI controller (candidate for USB mass-storage backend).
pub fn find_usb_xhci_controller() -> Option<UsbMsdInfo> {
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

                let class_code = pci_cfg_read32(addr, offset::CLASS_CODE);
                let class = (class_code >> 8) & 0xFFFF;
                if class != PCI_CLASS_USB_XHCI {
                    continue;
                }

                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
                let bar0 = pci_cfg_read32(addr, offset::BAR0);
                if bar0 == 0 || (bar0 & 0x01) != 0 {
                    continue;
                }

                let is_64bit = (bar0 & 0x06) == 0x04;
                let mmio_base = if is_64bit {
                    let bar1 = pci_cfg_read32(addr, offset::BAR1);
                    ((bar1 as u64) << 32) | ((bar0 & 0xFFFFFFF0) as u64)
                } else {
                    (bar0 & 0xFFFFFFF0) as u64
                };

                return Some(UsbMsdInfo {
                    pci_addr: addr,
                    mmio_base,
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

/// Intel AHCI quirk: firmware sometimes leaves PCS port-enable bits unset.
/// Mirror PORTS_IMPL into PCS_6 so AHCI ports are actually visible to software.
fn intel_ahci_pcs_quirk(addr: PciAddr, abar: u64) {
    const PCS_6: u8 = 0x92;
    const PCS_7: u8 = 0x94;
    const AHCI_CAP_OFF: u64 = 0x00;
    const AHCI_PI_OFF: u64 = 0x0C;

    if pci_cfg_read16(addr, offset::VENDOR_ID) != INTEL_VENDOR_ID {
        return;
    }

    let mut port_map = unsafe { core::ptr::read_volatile((abar + AHCI_PI_OFF) as *const u32) };
    if port_map == 0 {
        let cap = unsafe { core::ptr::read_volatile((abar + AHCI_CAP_OFF) as *const u32) };
        let n_ports = ((cap & 0x1F) + 1).min(32);
        port_map = if n_ports >= 32 {
            u32::MAX
        } else {
            (1u32 << n_ports) - 1
        };
    }

    if port_map == 0 {
        return;
    }

    let lo = (port_map & 0xFFFF) as u16;
    if lo != 0 {
        let pcs6 = pci_cfg_read16(addr, PCS_6);
        if (pcs6 & lo) != lo {
            pci_cfg_write16(addr, PCS_6, pcs6 | lo);
        }
    }

    let hi = ((port_map >> 16) & 0xFFFF) as u16;
    if hi != 0 {
        let pcs7 = pci_cfg_read16(addr, PCS_7);
        if (pcs7 & hi) != hi {
            pci_cfg_write16(addr, PCS_7, pcs7 | hi);
        }
    }
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
            intel_ahci_pcs_quirk(info.pci_addr, info.abar);

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

        DetectedBlockDevice::Sdhci(info) => {
            enable_pci_device(info.pci_addr);

            let sdhci_config = SdhciConfig {
                tsc_freq: config.tsc_freq,
                dma_phys: 0,
                dma_size: 0,
            };

            let driver = SdhciDriver::new(info.mmio_base, sdhci_config)?;
            Ok(BlockProbeResult::Sdhci(driver))
        }

        DetectedBlockDevice::UsbMsd(info) => {
            enable_pci_device(info.pci_addr);

            let usb_config = UsbMsdConfig {
                tsc_freq: config.tsc_freq,
                dma_phys: 0,
                dma_size: 0,
            };

            let driver = UsbMsdDriver::new(info.mmio_base, usb_config)?;
            Ok(BlockProbeResult::UsbMsd(driver))
        }

        DetectedBlockDevice::VirtIO {
            pci_addr,
            mmio_base,
        } => {
            // Enable device
            enable_pci_device(pci_addr);

            let blk_config = VirtioBlkConfig {
                queue_size: config.queue_size,
                desc_phys: config.virtio_desc_phys,
                avail_phys: config.virtio_avail_phys,
                used_phys: config.virtio_used_phys,
                headers_phys: config.virtio_headers_phys,
                status_phys: config.virtio_status_phys,
                headers_cpu: config.virtio_headers_cpu as u64,
                status_cpu: config.virtio_status_cpu as u64,
                notify_addr: 0,
                transport_type: 0,
            };

            // Try PCI Modern transport first (required for disable-legacy=on)
            let caps = probe_virtio_caps(pci_addr);
            if caps.has_required() {
                let pci_cfg = PciModernConfig {
                    common_cfg: caps.common_cfg_addr().unwrap_or(0),
                    notify_cfg: caps.notify_addr().unwrap_or(0),
                    notify_off_multiplier: caps.notify_multiplier(),
                    isr_cfg: caps.isr_addr().unwrap_or(0),
                    device_cfg: caps.device_cfg_addr().unwrap_or(0),
                    pci_cfg: 0,
                };
                let transport = VirtioTransport::pci_modern(pci_cfg);
                let driver =
                    VirtioBlkDriver::new_with_transport(transport, blk_config, config.tsc_freq)?;
                Ok(BlockProbeResult::VirtIO(driver))
            } else {
                // Fallback to legacy MMIO transport
                let mut legacy_config = blk_config;
                legacy_config.notify_addr = config.virtio_notify_addr;
                let driver = VirtioBlkDriver::new(mmio_base, legacy_config)?;
                Ok(BlockProbeResult::VirtIO(driver))
            }
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
        Ok(BlockProbeResult::Sdhci(driver)) => Ok(UnifiedBlockDevice::Sdhci(driver)),
        Ok(BlockProbeResult::UsbMsd(driver)) => Ok(UnifiedBlockDevice::UsbMsd(driver)),
        Err(BlockProbeError::NoDevice) => Err(UnifiedBlockError::NoDevice),
        Err(BlockProbeError::VirtioInitFailed) => Err(UnifiedBlockError::NoDevice),
        Err(BlockProbeError::AhciInitFailed) => Err(UnifiedBlockError::NoDevice),
        Err(BlockProbeError::SdhciInitFailed) => Err(UnifiedBlockError::NoDevice),
        Err(BlockProbeError::UsbMsdInitFailed) => Err(UnifiedBlockError::NoDevice),
        Err(_) => Err(UnifiedBlockError::NoDevice),
    }
}

/// Create a unified block device from a specific detected device.
///
/// Use with `scan_all_block_devices()` to iterate through devices
/// and initialize the one you want.
///
/// # Safety
/// Same as `probe_unified_block_device`.
pub unsafe fn create_unified_from_detected(
    detected: &DetectedBlockDevice,
    config: &BlockDmaConfig,
) -> Result<UnifiedBlockDevice, UnifiedBlockError> {
    match *detected {
        DetectedBlockDevice::Ahci(info) => {
            enable_pci_device(info.pci_addr);
            intel_ahci_pcs_quirk(info.pci_addr, info.abar);
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
            let driver = AhciDriver::new(info.abar, ahci_config).map_err(UnifiedBlockError::AhciError)?;
            Ok(UnifiedBlockDevice::Ahci(driver))
        }
        DetectedBlockDevice::Sdhci(info) => {
            enable_pci_device(info.pci_addr);
            let sdhci_config = SdhciConfig {
                tsc_freq: config.tsc_freq,
                dma_phys: 0,
                dma_size: 0,
            };
            let driver = SdhciDriver::new(info.mmio_base, sdhci_config)
                .map_err(UnifiedBlockError::SdhciError)?;
            Ok(UnifiedBlockDevice::Sdhci(driver))
        }
        DetectedBlockDevice::UsbMsd(info) => {
            enable_pci_device(info.pci_addr);
            let usb_config = UsbMsdConfig {
                tsc_freq: config.tsc_freq,
                dma_phys: 0,
                dma_size: 0,
            };
            let driver = UsbMsdDriver::new(info.mmio_base, usb_config)
                .map_err(UnifiedBlockError::UsbMsdError)?;
            Ok(UnifiedBlockDevice::UsbMsd(driver))
        }
        DetectedBlockDevice::VirtIO {
            pci_addr,
            mmio_base,
        } => {
            enable_pci_device(pci_addr);
            dbg_str("[BLK-PROBE] VirtIO pci=");
            dbg_hex8(pci_addr.bus);
            dbg_str(":");
            dbg_hex8(pci_addr.device);
            dbg_str(".");
            dbg_hex8(pci_addr.function);
            dbg_str("  bar0=");
            dbg_hex64(mmio_base);
            dbg_str("\n");

            // ── Raw PCI config diagnostics (bypass cap walker ASM) ──
            let status = pci_cfg_read16(pci_addr, 0x06);
            dbg_str("[BLK-PROBE] status=");
            dbg_hex32(status as u32);
            dbg_str(" cap_list_bit=");
            dbg_str(if status & 0x10 != 0 { "yes" } else { "no" });
            dbg_str("\n");
            if status & 0x10 != 0 {
                let cap_ptr = pci_cfg_read16(pci_addr, 0x34) as u8 & 0xFC;
                dbg_str("[BLK-PROBE] cap_ptr=0x");
                dbg_hex8(cap_ptr);
                dbg_str("\n");
                // Walk chain manually in Rust
                let mut ptr = cap_ptr;
                let mut walk = 0u32;
                while ptr != 0 && walk < 48 {
                    walk += 1;
                    let hdr = pci_cfg_read16(pci_addr, ptr);
                    let cap_id = (hdr & 0xFF) as u8;
                    let next = ((hdr >> 8) & 0xFC) as u8;
                    dbg_str("[BLK-PROBE]   cap@0x");
                    dbg_hex8(ptr);
                    dbg_str(" id=0x");
                    dbg_hex8(cap_id);
                    if cap_id == 0x09 {
                        // VirtIO vendor-specific: read cfg_type at ptr+3
                        let cfg_type = pci_cfg_read16(pci_addr, ptr + 2);
                        let cfg_type_byte = ((cfg_type >> 8) & 0xFF) as u8;
                        let bar_idx_raw = pci_cfg_read16(pci_addr, ptr + 4);
                        let bar_idx = (bar_idx_raw & 0xFF) as u8;
                        dbg_str(" VIRTIO cfg_type=");
                        dbg_hex8(cfg_type_byte);
                        dbg_str(" bar=");
                        dbg_hex8(bar_idx);
                        let bar_off = pci_cfg_read32(pci_addr, ptr + 8);
                        dbg_str(" off=");
                        dbg_hex32(bar_off);
                        let bar_len = pci_cfg_read32(pci_addr, ptr + 12);
                        dbg_str(" len=");
                        dbg_hex32(bar_len);
                    }
                    dbg_str(" next=0x");
                    dbg_hex8(next);
                    dbg_str("\n");
                    ptr = next;
                }
            }

            let blk_config = VirtioBlkConfig {
                queue_size: config.queue_size,
                desc_phys: config.virtio_desc_phys,
                avail_phys: config.virtio_avail_phys,
                used_phys: config.virtio_used_phys,
                headers_phys: config.virtio_headers_phys,
                status_phys: config.virtio_status_phys,
                headers_cpu: config.virtio_headers_cpu as u64,
                status_cpu: config.virtio_status_cpu as u64,
                notify_addr: 0, // determined by transport
                transport_type: 0,
            };

            // Try PCI Modern transport first (required for disable-legacy=on)
            let caps = probe_virtio_caps(pci_addr);
            dbg_str("[BLK-PROBE] caps found_mask=0x");
            dbg_hex8(caps.found_mask);
            dbg_str(" has_required=");
            dbg_str(if caps.has_required() { "yes" } else { "no" });
            dbg_str("\n");

            // Raw PCI config space dump (BARs + Command register)
            {
                let cmd = pci_cfg_read16(pci_addr, 0x04);
                dbg_str("[BLK-PROBE] PCI CMD=");
                dbg_hex32(cmd as u32);
                dbg_str(" (MEM_EN=");
                dbg_str(if cmd & 0x02 != 0 { "Y" } else { "N" });
                dbg_str(" BUS_MASTER=");
                dbg_str(if cmd & 0x04 != 0 { "Y" } else { "N" });
                dbg_str(")\n");
                for bar_i in 0..6u8 {
                    let raw = pci_cfg_read32(pci_addr, 0x10 + bar_i * 4);
                    dbg_str("[BLK-PROBE] raw BAR");
                    dbg_hex8(bar_i);
                    dbg_str("=");
                    dbg_hex32(raw);
                    dbg_str("\n");
                }
            }

            if caps.common.is_some() {
                dbg_str("[BLK-PROBE]   common_cfg=");
                dbg_hex64(caps.common_cfg_addr().unwrap_or(0));
                dbg_str("\n");
            }
            if caps.notify.is_some() {
                dbg_str("[BLK-PROBE]   notify_cfg=");
                dbg_hex64(caps.notify_addr().unwrap_or(0));
                dbg_str("  multiplier=");
                dbg_hex32(caps.notify_multiplier());
                dbg_str("\n");
            }
            if caps.device.is_some() {
                dbg_str("[BLK-PROBE]   device_cfg=");
                dbg_hex64(caps.device_cfg_addr().unwrap_or(0));
                dbg_str("\n");
            }
            if caps.isr.is_some() {
                dbg_str("[BLK-PROBE]   isr_cfg=");
                dbg_hex64(caps.isr_addr().unwrap_or(0));
                dbg_str("\n");
            }
            for i in 0..6 {
                if caps.bar_addrs[i] != 0 {
                    dbg_str("[BLK-PROBE]   BAR");
                    dbg_hex8(i as u8);
                    dbg_str("=");
                    dbg_hex64(caps.bar_addrs[i]);
                    dbg_str("\n");
                }
            }

            if caps.has_required() {
                let pci_cfg = PciModernConfig {
                    common_cfg: caps.common_cfg_addr().unwrap_or(0),
                    notify_cfg: caps.notify_addr().unwrap_or(0),
                    notify_off_multiplier: caps.notify_multiplier(),
                    isr_cfg: caps.isr_addr().unwrap_or(0),
                    device_cfg: caps.device_cfg_addr().unwrap_or(0),
                    pci_cfg: 0,
                };
                let transport = VirtioTransport::pci_modern(pci_cfg);

                // ── Feature-read diagnostic (volatile MMIO) ──
                {
                    let base = pci_cfg.common_cfg as *mut u32;
                    unsafe {
                        // Write ACKNOWLEDGE (0x01) to device_status to test MMIO write path
                        let status_ptr = (pci_cfg.common_cfg + 0x14) as *mut u8;
                        core::ptr::write_volatile(status_ptr, 0x00u8); // reset
                        core::arch::x86_64::_mm_mfence();
                        let st0 = core::ptr::read_volatile(status_ptr);
                        core::ptr::write_volatile(status_ptr, 0x01u8); // ACKNOWLEDGE
                        core::arch::x86_64::_mm_mfence();
                        let st1 = core::ptr::read_volatile(status_ptr);
                        core::ptr::write_volatile(status_ptr, 0x00u8); // reset again
                        core::arch::x86_64::_mm_mfence();
                        dbg_str("[BLK-PROBE] MMIO status write test: reset=");
                        dbg_hex8(st0);
                        dbg_str(" ack=");
                        dbg_hex8(st1);
                        dbg_str("\n");

                        // device_feature_select = 0, read device_feature (low 32)
                        core::ptr::write_volatile(base.add(0), 0u32); // offset 0x00
                        core::arch::x86_64::_mm_mfence();
                        let low = core::ptr::read_volatile(base.add(1)); // offset 0x04
                                                                         // device_feature_select = 1, read device_feature (high 32)
                        core::ptr::write_volatile(base.add(0), 1u32);
                        core::arch::x86_64::_mm_mfence();
                        let high = core::ptr::read_volatile(base.add(1));
                        let feats = ((high as u64) << 32) | (low as u64);
                        dbg_str("[BLK-PROBE] MMIO device_features low=");
                        dbg_hex32(low);
                        dbg_str(" high=");
                        dbg_hex32(high);
                        dbg_str(" combined=");
                        dbg_hex64(feats);
                        dbg_str("\n");
                    }
                }

                dbg_str("[BLK-PROBE] trying PCI Modern init...\n");
                match VirtioBlkDriver::new_with_transport(transport, blk_config, config.tsc_freq) {
                    Ok(driver) => {
                        dbg_str("[BLK-PROBE] PCI Modern init OK!\n");
                        Ok(UnifiedBlockDevice::VirtIO(driver))
                    }
                    Err(e) => {
                        dbg_str("[BLK-PROBE] PCI Modern init FAILED: ");
                        match e {
                            VirtioBlkInitError::ResetFailed => dbg_str("ResetFailed"),
                            VirtioBlkInitError::FeatureNegotiationFailed => {
                                dbg_str("FeatureNegotiationFailed")
                            }
                            VirtioBlkInitError::QueueSetupFailed => dbg_str("QueueSetupFailed"),
                            VirtioBlkInitError::DeviceFailed => dbg_str("DeviceFailed"),
                            VirtioBlkInitError::InvalidConfig => dbg_str("InvalidConfig"),
                            VirtioBlkInitError::TransportError => dbg_str("TransportError"),
                        }
                        dbg_str("\n");
                        Err(UnifiedBlockError::VirtioError(e))
                    }
                }
            } else {
                dbg_str("[BLK-PROBE] no PCI Modern caps, trying MMIO fallback...\n");
                // Fallback to legacy MMIO transport
                let mut legacy_config = blk_config;
                legacy_config.notify_addr = config.virtio_notify_addr;
                legacy_config.transport_type = 0;
                let driver = VirtioBlkDriver::new(mmio_base, legacy_config)
                    .map_err(UnifiedBlockError::VirtioError)?;
                Ok(UnifiedBlockDevice::VirtIO(driver))
            }
        }
    }
}

/// Create a unified block device from a specific AHCI port on a detected controller.
///
/// Returns `NoDevice` for non-AHCI devices.
///
/// # Safety
/// Same as `probe_unified_block_device`.
pub unsafe fn create_unified_from_detected_ahci_port(
    detected: &DetectedBlockDevice,
    config: &BlockDmaConfig,
    port_num: u32,
) -> Result<UnifiedBlockDevice, UnifiedBlockError> {
    match *detected {
        DetectedBlockDevice::Ahci(info) => {
            enable_pci_device(info.pci_addr);
            intel_ahci_pcs_quirk(info.pci_addr, info.abar);
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
            let driver = AhciDriver::new_on_port(info.abar, ahci_config, port_num)
                .map_err(UnifiedBlockError::AhciError)?;
            Ok(UnifiedBlockDevice::Ahci(driver))
        }
        _ => Err(UnifiedBlockError::NoDevice),
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
