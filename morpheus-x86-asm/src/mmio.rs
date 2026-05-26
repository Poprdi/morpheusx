//! MMIO loads / stores. Volatile semantics provided by the asm side; the
//! Rust wrappers are `#[inline]` and rely on the function-call boundary as
//! the compiler's reorder fence.
//!
//! # Safety
//! For every `read*`/`write*`:
//! - Address must be a valid MMIO mapping.
//! - Address must be naturally aligned for the access width.

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_mmio_read8(addr: u64) -> u8;
    fn asm_mmio_write8(addr: u64, value: u8);
    fn asm_mmio_read16(addr: u64) -> u16;
    fn asm_mmio_write16(addr: u64, value: u16);
    fn asm_mmio_read32(addr: u64) -> u32;
    fn asm_mmio_write32(addr: u64, value: u32);
}

/// Read 8-bit MMIO.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read8(addr: u64) -> u8 {
    asm_mmio_read8(addr)
}

/// Write 8-bit MMIO.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write8(addr: u64, value: u8) {
    asm_mmio_write8(addr, value)
}

/// Read 16-bit MMIO. Addr must be 2-byte aligned.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read16(addr: u64) -> u16 {
    asm_mmio_read16(addr)
}

/// Write 16-bit MMIO. Addr must be 2-byte aligned.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write16(addr: u64, value: u16) {
    asm_mmio_write16(addr, value)
}

/// Read 32-bit MMIO. Addr must be 4-byte aligned.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read32(addr: u64) -> u32 {
    asm_mmio_read32(addr)
}

/// Write 32-bit MMIO. Addr must be 4-byte aligned.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write32(addr: u64, value: u32) {
    asm_mmio_write32(addr, value)
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn read8(_addr: u64) -> u8 {
    0
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn write8(_addr: u64, _value: u8) {}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn read16(_addr: u64) -> u16 {
    0
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn write16(_addr: u64, _value: u16) {}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn read32(_addr: u64) -> u32 {
    0
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn write32(_addr: u64, _value: u32) {}
