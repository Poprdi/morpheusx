//! Intel e1000e network driver.
//!
//! Supports Intel I218-LM, 82579, and compatible NICs.
//!
//! # Supported Devices
//! - I218-LM (0x1502) - ThinkPad T450s
//! - I218-V (0x1503) - Consumer variant
//! - 82574L (0x10D3) - QEMU e1000e emulation
//! - 82579LM (0x1502) - Earlier ThinkPads
//!
//! # Reference
//! Intel 82579 Datasheet, Section 10 (Programming Interface)

pub mod e1000e;
pub mod init;
pub mod phy;
pub mod regs;
pub mod rx;
pub mod tx;

// Re-exports
pub use e1000e::{E1000eDriver, E1000eError};
pub use init::{E1000eConfig, E1000eInitError};

/// Intel PCI Vendor ID.
pub const INTEL_VENDOR_ID: u16 = 0x8086;

/// Supported e1000e device IDs.
///
/// This list covers the most common Intel GbE controllers found in
/// ThinkPads and QEMU emulation.
pub const E1000E_DEVICE_IDS: &[u16] = &[
    0x1502, // I218-LM (ThinkPad T450s, T440s, X240, etc.)
    0x1503, // I218-V (Consumer variant)
    0x10D3, // 82574L (QEMU e1000e emulation)
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

/// Check if a PCI device is a supported Intel e1000e NIC.
#[inline]
pub fn is_supported_device(vendor_id: u16, device_id: u16) -> bool {
    vendor_id == INTEL_VENDOR_ID && E1000E_DEVICE_IDS.contains(&device_id)
}

/// PCI class code for Ethernet controller.
pub const PCI_CLASS_NETWORK_ETHERNET: u32 = 0x020000;

/// Mask for PCI class code (ignore revision).
pub const PCI_CLASS_MASK: u32 = 0xFFFF00;

use crate::pci::config::{offset, pci_cfg_read16, pci_cfg_read32, PciAddr};

/// Information about a discovered Intel NIC.
#[derive(Debug, Clone, Copy)]
pub struct IntelNicInfo {
    /// PCI address (bus/device/function).
    pub pci_addr: PciAddr,
    /// PCI device ID.
    pub device_id: u16,
    /// BAR0 MMIO base address.
    pub mmio_base: u64,
    /// BAR0 size (from BAR sizing).
    pub mmio_size: u32,
}

/// Scan PCI bus for Intel e1000e NICs.
///
/// Returns the first supported device found, or None.
pub fn find_intel_nic() -> Option<IntelNicInfo> {
    // Scan all buses, devices, functions
    for bus in 0..=255u8 {
        for device in 0..32u8 {
            for function in 0..8u8 {
                let addr = PciAddr::new(bus, device, function);

                // Read vendor ID
                let vendor_id = pci_cfg_read16(addr, offset::VENDOR_ID);
                if vendor_id == 0xFFFF {
                    // No device present
                    if function == 0 {
                        break; // No device at this slot
                    }
                    continue;
                }

                // Check if it's an Intel NIC
                if vendor_id != INTEL_VENDOR_ID {
                    if function == 0 {
                        // Check header type for multi-function
                        let header = pci_cfg_read16(addr, offset::HEADER_TYPE) & 0x80;
                        if header == 0 {
                            break; // Single-function device
                        }
                    }
                    continue;
                }

                // Read device ID
                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
                if !E1000E_DEVICE_IDS.contains(&device_id) {
                    continue;
                }

                // Verify class code (Network Controller - Ethernet)
                let class_code = pci_cfg_read32(addr, offset::CLASS_CODE);
                if (class_code & PCI_CLASS_MASK) != PCI_CLASS_NETWORK_ETHERNET {
                    continue;
                }

                // Read BAR0
                let bar0 = pci_cfg_read32(addr, offset::BAR0);

                // Check BAR type (must be MMIO, not I/O)
                if bar0 & 0x01 != 0 {
                    // I/O space BAR - skip (we need MMIO)
                    continue;
                }

                // Check if 64-bit BAR
                let is_64bit = (bar0 & 0x06) == 0x04;
                let mmio_base = if is_64bit {
                    let bar1 = pci_cfg_read32(addr, offset::BAR1);
                    ((bar1 as u64) << 32) | ((bar0 & 0xFFFFFFF0) as u64)
                } else {
                    (bar0 & 0xFFFFFFF0) as u64
                };

                // Size BAR0 (write all 1s, read back, restore)
                let mmio_size = size_bar(addr, offset::BAR0);

                return Some(IntelNicInfo {
                    pci_addr: addr,
                    device_id,
                    mmio_base,
                    mmio_size,
                });
            }
        }
    }

    None
}

/// Size a BAR by writing all 1s and reading back.
fn size_bar(addr: PciAddr, bar_offset: u8) -> u32 {
    use crate::pci::config::pci_cfg_write32;

    // Save original value
    let original = pci_cfg_read32(addr, bar_offset);

    // Write all 1s
    pci_cfg_write32(addr, bar_offset, 0xFFFFFFFF);

    // Read back
    let sized = pci_cfg_read32(addr, bar_offset);

    // Restore original
    pci_cfg_write32(addr, bar_offset, original);

    // Calculate size (invert, mask type bits, add 1)
    if sized == 0 || sized == 0xFFFFFFFF {
        0
    } else {
        let mask = sized & 0xFFFFFFF0;
        (!mask).wrapping_add(1)
    }
}

/// Enable bus mastering and memory space access for the device.
pub fn enable_device(addr: PciAddr) {
    use crate::pci::config::pci_cfg_write16;

    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    // Set bits: Memory Space Enable (1), Bus Master Enable (2)
    let new_cmd = cmd | 0x06;
    pci_cfg_write16(addr, offset::COMMAND, new_cmd);
}

/// Validate MMIO access by reading STATUS register.
///
/// Returns true if the device appears responsive.
pub fn validate_mmio_access(mmio_base: u64) -> bool {
    use crate::asm::core::mmio::read32;

    // STATUS register is at offset 0x0008
    const STATUS_OFFSET: u64 = 0x0008;

    let status = unsafe { read32(mmio_base + STATUS_OFFSET) };

    // Check for valid status:
    // - Not all 1s (device not present)
    // - Not all 0s (device not responding)
    // - FD bit (bit 0) or LU bit (bit 1) should be reasonable
    status != 0xFFFFFFFF && status != 0x00000000
}
