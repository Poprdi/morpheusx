//! Shared kernel time source: one monotonic clock (TSC-derived, never steps back)
//! underlies SYS_CLOCK/SYS_CLOCK_GETTIME/SYS_NANOSLEEP and epoll/futex deadlines.
//! Wall time is the boot RTC anchor + monotonic delta, so CLOCK_REALTIME never
//! steps back and needs no per-call CMOS poll.

use crate::hal;
use core::sync::atomic::{AtomicU64, Ordering};

/// Monotonic ns captured at the instant the wall-clock anchor was taken.
static ANCHOR_MONO_NS: AtomicU64 = AtomicU64::new(0);
/// Wall-clock ns (Unix epoch) at that same instant. Both default to 0 so an
/// RTC-less machine degrades to 1970+uptime instead of faulting.
static ANCHOR_UNIX_NS: AtomicU64 = AtomicU64::new(0);

/// Monotonic nanoseconds since boot (canonical source); 0 until the TSC is calibrated.
#[inline]
pub fn monotonic_ns() -> u64 {
    hal().timer().now_ns()
}

/// Wall-clock nanoseconds (Unix epoch): `anchor + (now_mono - anchor_mono)`, so it
/// inherits monotonic's never-steps-back. Pre-anchor it reads 1970+uptime.
#[inline]
pub fn realtime_ns() -> u64 {
    let mono = monotonic_ns();
    let anchor_mono = ANCHOR_MONO_NS.load(Ordering::Relaxed);
    let anchor_unix = ANCHOR_UNIX_NS.load(Ordering::Relaxed);
    anchor_unix.saturating_add(mono.saturating_sub(anchor_mono))
}

/// Pin wall time to a boot RTC reading: pair `unix_secs` with the current monotonic
/// instant; later realtime reads extrapolate from it. A second call re-anchors.
pub fn anchor_realtime_unix_secs(unix_secs: u64) {
    let mono = monotonic_ns();
    // Store the monotonic side LAST: a concurrent realtime_ns() that observes the
    // new unix anchor against the old (smaller) mono base only over-counts by the
    // sub-call delta, never produces a backward step.
    ANCHOR_UNIX_NS.store(unix_secs.saturating_mul(1_000_000_000), Ordering::Relaxed);
    ANCHOR_MONO_NS.store(mono, Ordering::Relaxed);
}

/// TSC deadline `ns_from_now` in the future for the block_sleep/futex/epoll timeout
/// machinery (all compare against `read_tsc()`). Saturates to u64::MAX (= "forever")
/// on overflow or an uncalibrated TSC, per the deadline convention the tick expects.
pub fn tsc_deadline_in_ns(ns_from_now: u64) -> u64 {
    let freq = crate::schedular::tsc_frequency();
    if freq == 0 {
        return u64::MAX;
    }
    // ticks = ns * freq / 1e9, widened so the multiply can't wrap.
    let ticks = (ns_from_now as u128 * freq as u128) / 1_000_000_000u128;
    let ticks = if ticks > u64::MAX as u128 {
        u64::MAX
    } else {
        ticks as u64
    };
    hal().timer().read_tsc().saturating_add(ticks)
}
