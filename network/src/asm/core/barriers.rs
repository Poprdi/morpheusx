//! Memory barrier bindings.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.2.1, §2.4

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_bar_sfence();
    fn asm_bar_lfence();
    fn asm_bar_mfence();
}

/// Store fence - ensures all prior stores are globally visible.
///
/// Use before device notification to ensure descriptors are written.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn sfence() {
    unsafe { asm_bar_sfence(); }
}

/// Load fence - ensures all prior loads complete before subsequent.
///
/// Use after reading device-written data to ensure consistency.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn lfence() {
    unsafe { asm_bar_lfence(); }
}

/// Full memory fence - ensures all prior loads AND stores complete.
///
/// Use when both ordering constraints needed (e.g., before MMIO notify).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn mfence() {
    unsafe { asm_bar_mfence(); }
}

// Stubs for non-x86_64
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn sfence() {}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn lfence() {}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn mfence() {}
