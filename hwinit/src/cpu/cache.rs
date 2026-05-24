//! CLFLUSH bindings.

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_cache_clflush(addr: u64);
    fn asm_cache_flush_range(addr: u64, len: u64);
}

/// CLFLUSH the cache line containing `addr`.
///
/// # Safety
/// `addr` must be valid.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn clflush(addr: *const u8) {
    asm_cache_clflush(addr as u64)
}

/// CLFLUSH a range. Use before DMA submit on non-UC buffers.
///
/// # Safety
/// Range must be valid.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn flush_range(addr: *const u8, len: usize) {
    asm_cache_flush_range(addr as u64, len as u64)
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn clflush(_addr: *const u8) {}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn flush_range(_addr: *const u8, _len: usize) {}
