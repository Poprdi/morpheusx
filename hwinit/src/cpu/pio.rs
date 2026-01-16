//! Port I/O bindings.
//!
//! # Safety
//! Port must be valid I/O port for the operation.
//!
//! # Reference
//! ARCHITECTURE_V3.md - PIO layer

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_pio_read8(port: u16) -> u8;
    fn asm_pio_write8(port: u16, value: u8);
    fn asm_pio_read16(port: u16) -> u16;
    fn asm_pio_write16(port: u16, value: u16);
    fn asm_pio_read32(port: u16) -> u32;
    fn asm_pio_write32(port: u16, value: u32);
}

/// Read 8-bit value from I/O port.
///
/// # Safety
/// Port must be valid and accessible.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    asm_pio_read8(port)
}

/// Write 8-bit value to I/O port.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    asm_pio_write8(port, value)
}

/// Read 16-bit value from I/O port.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn inw(port: u16) -> u16 {
    asm_pio_read16(port)
}

/// Write 16-bit value to I/O port.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn outw(port: u16, value: u16) {
    asm_pio_write16(port, value)
}

/// Read 32-bit value from I/O port.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn inl(port: u16) -> u32 {
    asm_pio_read32(port)
}

/// Write 32-bit value to I/O port.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn outl(port: u16, value: u32) {
    asm_pio_write32(port, value)
}

// Stubs for non-x86_64
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn inb(_port: u16) -> u8 {
    0
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn outb(_port: u16, _value: u8) {}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn inw(_port: u16) -> u16 {
    0
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn outw(_port: u16, _value: u16) {}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn inl(_port: u16) -> u32 {
    0
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn outl(_port: u16, _value: u32) {}
