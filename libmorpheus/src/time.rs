//! Time — monotonic clock access.

use crate::raw::*;

/// Get monotonic nanoseconds since boot.
///
/// Derived from the TSC (Time Stamp Counter).  Returns 0 if the TSC
/// has not been calibrated.
pub fn clock_gettime() -> u64 {
    unsafe { syscall0(SYS_CLOCK) }
}

/// Get monotonic milliseconds since boot (convenience wrapper).
pub fn uptime_ms() -> u64 {
    clock_gettime() / 1_000_000
}

/// Get monotonic microseconds since boot (convenience wrapper).
pub fn uptime_us() -> u64 {
    clock_gettime() / 1_000
}
