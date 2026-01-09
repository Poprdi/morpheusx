//! MMIO (Memory-Mapped I/O) bindings.
//!
//! # Safety
//! - Address must be valid MMIO address
//! - Address must be properly aligned
//! - Address must be mapped with appropriate attributes
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.2.1

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_mmio_read8(addr: u64) -> u8;
    fn asm_mmio_write8(addr: u64, value: u8);
    fn asm_mmio_read16(addr: u64) -> u16;
    fn asm_mmio_write16(addr: u64, value: u16);
    fn asm_mmio_read32(addr: u64) -> u32;
    fn asm_mmio_write32(addr: u64, value: u32);
}

/// Read 8-bit value from MMIO address.
///
/// # Safety
/// Address must be valid MMIO address.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read8(addr: u64) -> u8 {
    asm_mmio_read8(addr)
}

/// Write 8-bit value to MMIO address.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write8(addr: u64, value: u8) {
    asm_mmio_write8(addr, value)
}

/// Read 16-bit value from MMIO address.
///
/// # Safety
/// Address must be valid, 2-byte aligned MMIO address.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read16(addr: u64) -> u16 {
    asm_mmio_read16(addr)
}

/// Write 16-bit value to MMIO address.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write16(addr: u64, value: u16) {
    asm_mmio_write16(addr, value)
}

/// Read 32-bit value from MMIO address.
///
/// # Safety
/// Address must be valid, 4-byte aligned MMIO address.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read32(addr: u64) -> u32 {
    asm_mmio_read32(addr)
}

/// Write 32-bit value to MMIO address.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write32(addr: u64, value: u32) {
    asm_mmio_write32(addr, value)
}

// Stubs for non-x86_64
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn read8(_addr: u64) -> u8 { 0 }
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn write8(_addr: u64, _value: u8) {}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn read16(_addr: u64) -> u16 { 0 }
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn write16(_addr: u64, _value: u16) {}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn read32(_addr: u64) -> u32 { 0 }
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn write32(_addr: u64, _value: u32) {}
