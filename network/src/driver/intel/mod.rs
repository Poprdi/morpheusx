//! Intel e1000e family driver (82574L, 82577/9, I218, I219). See 82579 datasheet §10.

pub mod e1000e;
pub mod init;
pub mod phy;
pub mod regs;
pub mod rx;
pub mod tx;

pub use e1000e::{E1000eDriver, E1000eError};
pub use init::{E1000eConfig, E1000eInitError};

pub const INTEL_VENDOR_ID: u16 = 0x8086;

pub const E1000E_DEVICE_IDS: &[u16] = &[
    0x100E, // 82540EM (QEMU e1000)
    0x1502, // I218-LM (ThinkPad T450s, T440s, X240, etc.)
    0x1503, // I218-V (Consumer variant)
    0x155A, // I218-LM (ThinkPad T450s variant)
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

#[inline]
pub fn is_supported_device(vendor_id: u16, device_id: u16) -> bool {
    vendor_id == INTEL_VENDOR_ID && E1000E_DEVICE_IDS.contains(&device_id)
}

pub const PCI_CLASS_NETWORK_ETHERNET: u32 = 0x0200_0000;
pub const PCI_CLASS_MASK: u32 = 0xFFFF_0000;

use crate::pci::config::{offset, pci_cfg_read16, pci_cfg_read32, PciAddr};

#[derive(Debug, Clone, Copy)]
pub struct IntelNicInfo {
    pub pci_addr: PciAddr,
    pub device_id: u16,
    pub mmio_base: u64,
    pub mmio_size: u32,
}

/// Returns the first matching e1000e device on the bus.
pub fn find_intel_nic() -> Option<IntelNicInfo> {
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

                if vendor_id != INTEL_VENDOR_ID {
                    if function == 0 {
                        // Skip remaining functions on single-function devices.
                        let header = pci_cfg_read16(addr, offset::HEADER_TYPE) & 0x80;
                        if header == 0 {
                            break;
                        }
                    }
                    continue;
                }

                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
                if !E1000E_DEVICE_IDS.contains(&device_id) {
                    continue;
                }

                let class_code = pci_cfg_read32(addr, offset::CLASS_CODE);
                if (class_code & PCI_CLASS_MASK) != PCI_CLASS_NETWORK_ETHERNET {
                    continue;
                }

                let bar0 = pci_cfg_read32(addr, offset::BAR0);

                if bar0 & 0x01 != 0 {
                    // I/O-space BAR; we need MMIO.
                    continue;
                }

                let is_64bit = (bar0 & 0x06) == 0x04;
                let mmio_base = if is_64bit {
                    let bar1 = pci_cfg_read32(addr, offset::BAR1);
                    ((bar1 as u64) << 32) | ((bar0 & 0xFFFFFFF0) as u64)
                } else {
                    (bar0 & 0xFFFFFFF0) as u64
                };

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

fn size_bar(addr: PciAddr, bar_offset: u8) -> u32 {
    use crate::pci::config::pci_cfg_write32;

    let original = pci_cfg_read32(addr, bar_offset);
    pci_cfg_write32(addr, bar_offset, 0xFFFFFFFF);
    let sized = pci_cfg_read32(addr, bar_offset);
    pci_cfg_write32(addr, bar_offset, original);

    if sized == 0 || sized == 0xFFFFFFFF {
        0
    } else {
        let mask = sized & 0xFFFFFFF0;
        (!mask).wrapping_add(1)
    }
}

/// Set PCI command MSE | BME.
pub fn enable_device(addr: PciAddr) {
    use crate::pci::config::pci_cfg_write16;

    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    let new_cmd = cmd | 0x06;
    pci_cfg_write16(addr, offset::COMMAND, new_cmd);
}

/// STATUS read; false on all-ones (no device) or all-zeros (dead).
pub fn validate_mmio_access(mmio_base: u64) -> bool {
    use crate::asm::core::mmio::read32;

    const STATUS_OFFSET: u64 = 0x0008;

    let status = unsafe { read32(mmio_base + STATUS_OFFSET) };
    status != 0xFFFFFFFF && status != 0x00000000
}
