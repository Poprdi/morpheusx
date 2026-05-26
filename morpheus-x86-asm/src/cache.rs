//! CLFLUSH / CLFLUSHOPT range flush.

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_cache_clflush(addr: u64);
    fn asm_cache_flush_range(addr: u64, len: u64);
}

/// CLFLUSH the cache line containing `addr`.
///
/// # Safety
/// `addr` must point to mapped, accessible memory.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn clflush(addr: *const u8) {
    // SAFETY: forwarded to caller; addr validity is the caller's contract.
    asm_cache_clflush(addr as u64)
}

/// CLFLUSHOPT over the range `[addr, addr + len)`, followed by SFENCE.
/// Use before DMA submit on non-UC buffers.
///
/// # Safety
/// `addr..addr+len` must be a single contiguous mapped region.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn flush_range(addr: *const u8, len: usize) {
    // SAFETY: forwarded to caller; range validity is the caller's contract.
    asm_cache_flush_range(addr as u64, len as u64)
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn clflush(_addr: *const u8) {}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn flush_range(_addr: *const u8, _len: usize) {}
