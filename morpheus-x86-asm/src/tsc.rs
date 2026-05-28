//! TSC reads. Always architecturally safe; requires invariant TSC for
//! cross-CPU comparison (verify via CPUID at boot, not here).

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_tsc_read() -> u64;
    fn asm_tsc_read_serialized() -> u64;
}

/// Read TSC (non-serializing, ~40 cycles). May be reordered.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn read_tsc() -> u64 {
    // SAFETY: RDTSC is unprivileged in user-mode (CR4.TSD=0 by default) and
    // unconditionally legal at CPL0; the wrapper has no side effects.
    unsafe { asm_tsc_read() }
}

/// Read TSC with CPUID serialization (~200 cycles). Prior instructions
/// guaranteed retired before the read.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn read_tsc_serialized() -> u64 {
    // SAFETY: CPUID + RDTSC; both legal at CPL0 with no memory side effects.
    unsafe { asm_tsc_read_serialized() }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn read_tsc() -> u64 {
    0
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn read_tsc_serialized() -> u64 {
    0
}
