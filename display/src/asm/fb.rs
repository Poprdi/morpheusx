//! Extern bindings for asm/fb.s. The call boundary acts as a compiler
//! barrier so framebuffer writes cannot be reordered. Same pattern as
//! network/asm/core/mmio.s.
//!
//! SAFETY (all functions): addresses within mapped fb, 4-byte aligned.

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_fb_write32(addr: u64, value: u32);
    fn asm_fb_read32(addr: u64) -> u32;
    fn asm_fb_memset32(addr: u64, value: u32, count: u64);
    fn asm_fb_memcpy(dst: u64, src: u64, bytes: u64);
}

#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn write32(addr: u64, value: u32) {
    asm_fb_write32(addr, value);
}

#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read32(addr: u64) -> u32 {
    asm_fb_read32(addr)
}

/// `count` is u32 elements, not bytes.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn memset32(addr: u64, value: u32, count: u64) {
    asm_fb_memset32(addr, value, count);
}

/// Forward copy; caller must ensure dst < src or non-overlapping.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn memcpy(dst: u64, src: u64, bytes: u64) {
    asm_fb_memcpy(dst, src, bytes);
}

// Non-x86_64 stubs for host builds.
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
