//! PCI config-space access via 0xCF8/0xCFC (asm thunks).

extern "win64" {
    fn asm_pci_cfg_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8;
    fn asm_pci_cfg_read16(bus: u8, device: u8, function: u8, offset: u8) -> u16;
    fn asm_pci_cfg_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32;
    fn asm_pci_cfg_write8(bus: u8, device: u8, function: u8, offset: u8, value: u8);
    fn asm_pci_cfg_write16(bus: u8, device: u8, function: u8, offset: u8, value: u16);
    fn asm_pci_cfg_write32(bus: u8, device: u8, function: u8, offset: u8, value: u32);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciAddr {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PciAddr {
    pub const fn new(bus: u8, device: u8, function: u8) -> Self {
        Self {
            bus,
            device,
            function,
        }
    }
}

#[inline]
pub fn pci_cfg_read8(addr: PciAddr, offset: u8) -> u8 {
    unsafe { asm_pci_cfg_read8(addr.bus, addr.device, addr.function, offset) }
}

#[inline]
pub fn pci_cfg_read16(addr: PciAddr, offset: u8) -> u16 {
    unsafe { asm_pci_cfg_read16(addr.bus, addr.device, addr.function, offset) }
}

#[inline]
pub fn pci_cfg_read32(addr: PciAddr, offset: u8) -> u32 {
    unsafe { asm_pci_cfg_read32(addr.bus, addr.device, addr.function, offset) }
}

#[inline]
pub fn pci_cfg_write8(addr: PciAddr, offset: u8, value: u8) {
    unsafe { asm_pci_cfg_write8(addr.bus, addr.device, addr.function, offset, value) }
}

#[inline]
pub fn pci_cfg_write16(addr: PciAddr, offset: u8, value: u16) {
    unsafe { asm_pci_cfg_write16(addr.bus, addr.device, addr.function, offset, value) }
}

#[inline]
pub fn pci_cfg_write32(addr: PciAddr, offset: u8, value: u32) {
    unsafe { asm_pci_cfg_write32(addr.bus, addr.device, addr.function, offset, value) }
}

/// Standard PCI config-space offsets (PCI 3.0 §6).
pub mod offset {
    pub const VENDOR_ID: u8 = 0x00;
    pub const DEVICE_ID: u8 = 0x02;
    pub const COMMAND: u8 = 0x04;
    pub const STATUS: u8 = 0x06;
    pub const REVISION_ID: u8 = 0x08;
    pub const CLASS_CODE: u8 = 0x09;
    pub const CACHE_LINE_SIZE: u8 = 0x0C;
    pub const LATENCY_TIMER: u8 = 0x0D;
    pub const HEADER_TYPE: u8 = 0x0E;
    pub const BIST: u8 = 0x0F;
    pub const BAR0: u8 = 0x10;
    pub const BAR1: u8 = 0x14;
    pub const BAR2: u8 = 0x18;
    pub const BAR3: u8 = 0x1C;
    pub const BAR4: u8 = 0x20;
    pub const BAR5: u8 = 0x24;
    pub const CARDBUS_CIS: u8 = 0x28;
    pub const SUBSYS_VENDOR_ID: u8 = 0x2C;
    pub const SUBSYS_ID: u8 = 0x2E;
    pub const ROM_BASE: u8 = 0x30;
    pub const CAP_PTR: u8 = 0x34;
    pub const INT_LINE: u8 = 0x3C;
    pub const INT_PIN: u8 = 0x3D;
}

pub mod status {
    pub const CAP_LIST: u16 = 1 << 4;
}
