//! PCI Legacy (CF8/CFC) configuration space bindings.
//!
//! Uses I/O ports 0xCF8 (address) and 0xCFC (data) for PCI config access.
//!
//! # Reference
//! ARCHITECTURE_V3.md - PCI layer

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_pci_legacy_read8(bus: u8, dev: u8, func: u8, reg: u8) -> u8;
    fn asm_pci_legacy_write8(bus: u8, dev: u8, func: u8, reg: u8, val: u8);
    fn asm_pci_legacy_read16(bus: u8, dev: u8, func: u8, reg: u8) -> u16;
    fn asm_pci_legacy_write16(bus: u8, dev: u8, func: u8, reg: u8, val: u16);
    fn asm_pci_legacy_read32(bus: u8, dev: u8, func: u8, reg: u8) -> u32;
    fn asm_pci_legacy_write32(bus: u8, dev: u8, func: u8, reg: u8, val: u32);
}

/// Read 8-bit value from PCI config space.
///
/// # Safety
/// Must have I/O port access privileges.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read8(bus: u8, dev: u8, func: u8, reg: u8) -> u8 {
    asm_pci_legacy_read8(bus, dev, func, reg)
}

/// Write 8-bit value to PCI config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write8(bus: u8, dev: u8, func: u8, reg: u8, val: u8) {
    asm_pci_legacy_write8(bus, dev, func, reg, val)
}

/// Read 16-bit value from PCI config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read16(bus: u8, dev: u8, func: u8, reg: u8) -> u16 {
    asm_pci_legacy_read16(bus, dev, func, reg)
}

/// Write 16-bit value to PCI config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write16(bus: u8, dev: u8, func: u8, reg: u8, val: u16) {
    asm_pci_legacy_write16(bus, dev, func, reg, val)
}

/// Read 32-bit value from PCI config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read32(bus: u8, dev: u8, func: u8, reg: u8) -> u32 {
    asm_pci_legacy_read32(bus, dev, func, reg)
}

/// Write 32-bit value to PCI config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write32(bus: u8, dev: u8, func: u8, reg: u8, val: u32) {
    asm_pci_legacy_write32(bus, dev, func, reg, val)
}

// Stubs for non-x86_64
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn read8(_bus: u8, _dev: u8, _func: u8, _reg: u8) -> u8 {
    0
}
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn write8(_bus: u8, _dev: u8, _func: u8, _reg: u8, _val: u8) {}
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn read16(_bus: u8, _dev: u8, _func: u8, _reg: u8) -> u16 {
    0
}
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn write16(_bus: u8, _dev: u8, _func: u8, _reg: u8, _val: u16) {}
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn read32(_bus: u8, _dev: u8, _func: u8, _reg: u8) -> u32 {
    0
}
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn write32(_bus: u8, _dev: u8, _func: u8, _reg: u8, _val: u32) {}
