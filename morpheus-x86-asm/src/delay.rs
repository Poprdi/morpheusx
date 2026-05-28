//! Blocking spin delays. Use ONLY during initialization — never from the
//! main poll loop, an ISR, or any code that could be holding a lock.

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_spin_hint();
    fn asm_delay_tsc(ticks: u64);
    fn asm_delay_us(us: u64, tsc_freq: u64);
}

/// `pause` — spin-loop hint.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn spin_hint() {
    // SAFETY: PAUSE has no operands and no side effects.
    unsafe {
        asm_spin_hint();
    }
}

/// Block for `ticks` TSC counts. Init-time only.
///
/// # Safety
/// Blocking call — caller must not hold any lock that another path needs.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn delay_tsc(ticks: u64) {
    // SAFETY: blocking-loop contract is the caller's per docstring.
    asm_delay_tsc(ticks)
}

/// Block for `us` microseconds given `tsc_freq` (Hz). Init-time only.
///
/// # Safety
/// Blocking call — caller must not hold any lock that another path needs.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn delay_us(us: u64, tsc_freq: u64) {
    // SAFETY: blocking-loop contract is the caller's per docstring.
    asm_delay_us(us, tsc_freq)
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn spin_hint() {}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn delay_tsc(_ticks: u64) {}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub unsafe fn delay_us(_us: u64, _tsc_freq: u64) {}
