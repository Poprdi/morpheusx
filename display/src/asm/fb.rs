//! Framebuffer ASM bindings.
//!
//! Provides extern declarations for asm/fb.s functions.
//! All framebuffer memory access goes through these - NO Rust volatile.
//!
//! # Safety
//! - Address must be within valid framebuffer region
//! - Address must be 4-byte aligned for 32-bit operations
//!
//! # Design
//! Standalone ASM function call acts as compiler barrier.
//! Compiler cannot reorder memory operations across the call.
//! This is the same pattern used by network/asm/core/mmio.s.

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_fb_write32(addr: u64, value: u32);
    fn asm_fb_read32(addr: u64) -> u32;
    fn asm_fb_memset32(addr: u64, value: u32, count: u64);
    fn asm_fb_memcpy(dst: u64, src: u64, bytes: u64);
}

/// Write a 32-bit pixel value to framebuffer address.
///
/// # Safety
/// Address must be within valid framebuffer region, 4-byte aligned.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write32(addr: u64, value: u32) {
    asm_fb_write32(addr, value);
}

/// Read a 32-bit pixel value from framebuffer address.
///
/// # Safety
/// Address must be within valid framebuffer region, 4-byte aligned.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read32(addr: u64) -> u32 {
    asm_fb_read32(addr)
}

/// Fill framebuffer region with 32-bit value.
///
/// # Arguments
/// - `addr`: Start address (must be 4-byte aligned)
/// - `value`: 32-bit pixel value to fill with
/// - `count`: Number of 32-bit values to write (NOT bytes)
///
/// # Safety
/// Region must be within valid framebuffer.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn memset32(addr: u64, value: u32, count: u64) {
    asm_fb_memset32(addr, value, count);
}

/// Copy memory within framebuffer (for scrolling).
///
/// # Arguments
/// - `dst`: Destination address
/// - `src`: Source address
/// - `bytes`: Number of bytes to copy
///
/// # Safety
/// Both regions must be within valid framebuffer.
/// For scroll up (dst < src), forward copy is safe.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn memcpy(dst: u64, src: u64, bytes: u64) {
    asm_fb_memcpy(dst, src, bytes);
}

// Stubs for non-x86_64 (host builds, tests)
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn write32(_addr: u64, _value: u32) {}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn read32(_addr: u64) -> u32 {
    0
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn memset32(_addr: u64, _value: u32, _count: u64) {}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn memcpy(_dst: u64, _src: u64, _bytes: u64) {}
