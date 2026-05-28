//! Memory barriers (SFENCE / LFENCE / MFENCE).

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_bar_sfence();
    fn asm_bar_lfence();
    fn asm_bar_mfence();
}

/// SFENCE. Use before a device doorbell to publish descriptor writes.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn sfence() {
    // SAFETY: SFENCE has no operands and no memory access; the wrapper symbol
    // is a leaf function that returns without touching caller state.
    unsafe {
        asm_bar_sfence();
    }
}

/// LFENCE. Use after reading device-written data.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn lfence() {
    // SAFETY: LFENCE has no operands and no memory access.
    unsafe {
        asm_bar_lfence();
    }
}

/// MFENCE. Use when both load and store ordering required.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn mfence() {
    // SAFETY: MFENCE has no operands and no memory access.
    unsafe {
        asm_bar_mfence();
    }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn sfence() {}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn lfence() {}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn mfence() {}
