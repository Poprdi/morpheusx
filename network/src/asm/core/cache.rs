//! Cache management bindings.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3.6 - Cache coherency

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_cache_clflush(addr: u64);
    fn asm_cache_flush_range(addr: u64, len: u64);
}

/// Flush single cache line containing address.
///
/// # Safety
/// Address must be valid.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn clflush(addr: *const u8) {
    asm_cache_clflush(addr as u64)
}

/// Flush cache lines for entire range.
///
/// Use before submitting DMA buffer to device (if not UC mapped).
///
/// # Safety
/// Address range must be valid.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn flush_range(addr: *const u8, len: usize) {
    asm_cache_flush_range(addr as u64, len as u64)
}

// Stubs for non-x86_64
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn clflush(_addr: *const u8) {}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn flush_range(_addr: *const u8, _len: usize) {}
