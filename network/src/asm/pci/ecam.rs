//! PCIe ECAM configuration space bindings.
//!
//! ECAM (Enhanced Configuration Access Mechanism) provides memory-mapped
//! access to the full 4KB PCIe configuration space.
//!
//! # Reference
//! ARCHITECTURE_V3.md - PCI layer

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Calculate ECAM address for a device register.
    fn asm_pci_ecam_addr(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16) -> u64;
    
    fn asm_pci_ecam_read8(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16) -> u8;
    fn asm_pci_ecam_write8(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16, val: u8);
    fn asm_pci_ecam_read16(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16) -> u16;
    fn asm_pci_ecam_write16(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16, val: u16);
    fn asm_pci_ecam_read32(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16) -> u32;
    fn asm_pci_ecam_write32(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16, val: u32);
}

/// Calculate ECAM address for accessing a register.
///
/// # Formula
/// addr = ecam_base + (bus << 20) + (dev << 15) + (func << 12) + reg
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn ecam_addr(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16) -> u64 {
    unsafe { asm_pci_ecam_addr(ecam_base, bus, dev, func, reg) }
}

/// Read 8-bit value from ECAM config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read8(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16) -> u8 {
    asm_pci_ecam_read8(ecam_base, bus, dev, func, reg)
}

/// Write 8-bit value to ECAM config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write8(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16, val: u8) {
    asm_pci_ecam_write8(ecam_base, bus, dev, func, reg, val)
}

/// Read 16-bit value from ECAM config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read16(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16) -> u16 {
    asm_pci_ecam_read16(ecam_base, bus, dev, func, reg)
}

/// Write 16-bit value to ECAM config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write16(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16, val: u16) {
    asm_pci_ecam_write16(ecam_base, bus, dev, func, reg, val)
}

/// Read 32-bit value from ECAM config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read32(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16) -> u32 {
    asm_pci_ecam_read32(ecam_base, bus, dev, func, reg)
}

/// Write 32-bit value to ECAM config space.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write32(ecam_base: u64, bus: u8, dev: u8, func: u8, reg: u16, val: u32) {
    asm_pci_ecam_write32(ecam_base, bus, dev, func, reg, val)
}

// Stubs for non-x86_64
#[cfg(not(target_arch = "x86_64"))]
pub fn ecam_addr(_ecam_base: u64, _bus: u8, _dev: u8, _func: u8, _reg: u16) -> u64 { 0 }
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn read8(_ecam_base: u64, _bus: u8, _dev: u8, _func: u8, _reg: u16) -> u8 { 0 }
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn write8(_ecam_base: u64, _bus: u8, _dev: u8, _func: u8, _reg: u16, _val: u8) {}
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn read16(_ecam_base: u64, _bus: u8, _dev: u8, _func: u8, _reg: u16) -> u16 { 0 }
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn write16(_ecam_base: u64, _bus: u8, _dev: u8, _func: u8, _reg: u16, _val: u16) {}
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn read32(_ecam_base: u64, _bus: u8, _dev: u8, _func: u8, _reg: u16) -> u32 { 0 }
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn write32(_ecam_base: u64, _bus: u8, _dev: u8, _func: u8, _reg: u16, _val: u32) {}
