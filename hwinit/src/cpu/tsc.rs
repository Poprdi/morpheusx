//! TSC (Time Stamp Counter) bindings.
//!
//! # Safety
//! TSC reads are always safe. Requires invariant TSC (verify via CPUID at boot).
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.2.1

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Read TSC (non-serializing, ~40 cycles).
    fn asm_tsc_read() -> u64;

    /// Read TSC with CPUID serialization (~200 cycles).
    fn asm_tsc_read_serialized() -> u64;
}

/// Read TSC (non-serializing).
///
/// Fast (~40 cycles) but may be reordered with surrounding instructions.
/// Use for timing intervals where exact ordering isn't critical.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn read_tsc() -> u64 {
    unsafe { asm_tsc_read() }
}

/// Read TSC with serialization.
///
/// Slower (~200 cycles) but guarantees all prior instructions complete before reading.
/// Use for precise measurement boundaries.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn read_tsc_serialized() -> u64 {
    unsafe { asm_tsc_read_serialized() }
}

/// Stub for non-x86_64 targets.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn read_tsc() -> u64 {
    0
}

/// Stub for non-x86_64 targets.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn read_tsc_serialized() -> u64 {
    0
}
