//! Port I/O (IN / OUT). IN/OUT are inherently serializing on x86.
//!
//! # Safety
//! For every `inb`/`outb`/etc.:
//! - `port` must be a valid I/O port for the operation.
//! - The device behind the port must tolerate the access pattern.

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_pio_read8(port: u16) -> u8;
    fn asm_pio_write8(port: u16, value: u8);
    fn asm_pio_read16(port: u16) -> u16;
    fn asm_pio_write16(port: u16, value: u16);
    fn asm_pio_read32(port: u16) -> u32;
    fn asm_pio_write32(port: u16, value: u32);
}

/// `in al, dx` — 8-bit port read.
///
/// # Safety
/// `port` must be a valid I/O port that tolerates an 8-bit read.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    asm_pio_read8(port)
}

/// `out dx, al` — 8-bit port write.
///
/// # Safety
/// `port` must be a valid I/O port that tolerates an 8-bit write.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    asm_pio_write8(port, value)
}

/// `in ax, dx` — 16-bit port read.
///
/// # Safety
/// `port` must be a valid I/O port that tolerates a 16-bit read.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn inw(port: u16) -> u16 {
    asm_pio_read16(port)
}

/// `out dx, ax` — 16-bit port write.
///
/// # Safety
/// `port` must be a valid I/O port that tolerates a 16-bit write.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn outw(port: u16, value: u16) {
    asm_pio_write16(port, value)
}

/// `in eax, dx` — 32-bit port read.
///
/// # Safety
/// `port` must be a valid I/O port that tolerates a 32-bit read.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn inl(port: u16) -> u32 {
    asm_pio_read32(port)
}

/// `out dx, eax` — 32-bit port write.
///
/// # Safety
/// `port` must be a valid I/O port that tolerates a 32-bit write.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn outl(port: u16, value: u32) {
    asm_pio_write32(port, value)
}

/// # Safety
/// `port` must be a valid I/O port for the operation.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn inb(_port: u16) -> u8 {
    0
}
/// # Safety
/// `port` must be a valid I/O port for the operation.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn outb(_port: u16, _value: u8) {}
/// # Safety
/// `port` must be a valid I/O port for the operation.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn inw(_port: u16) -> u16 {
    0
}
/// # Safety
/// `port` must be a valid I/O port for the operation.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn outw(_port: u16, _value: u16) {}
/// # Safety
/// `port` must be a valid I/O port for the operation.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn inl(_port: u16) -> u32 {
    0
}
/// # Safety
/// `port` must be a valid I/O port for the operation.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn outl(_port: u16, _value: u32) {}
