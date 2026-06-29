// clock_gettime + nanosleep over the shared kernel time source (`crate::clock`),
// the same source the scalar SYS_CLOCK(22) reads so the two paths can't drift.
// CLOCK_MONOTONIC is boot-relative; CLOCK_REALTIME is the boot RTC anchor.

use super::common::*;
use morpheus_foundation::flags::{CLOCK_MONOTONIC, CLOCK_REALTIME};
use morpheus_foundation::types::Timespec;

const NANOS_PER_SEC: u64 = 1_000_000_000;

#[inline]
fn ns_to_timespec(ns: u64) -> Timespec {
    Timespec {
        tv_sec: (ns / NANOS_PER_SEC) as i64,
        tv_nsec: (ns % NANOS_PER_SEC) as i64,
    }
}

/// SYS_CLOCK_GETTIME: `clock_id,*mut Timespec -> 0 | -errno`.
pub unsafe fn sys_clock_gettime(clock_id: u64, ts_ptr: u64) -> u64 {
    if !validate_user_buf(ts_ptr, core::mem::size_of::<Timespec>() as u64) {
        return EFAULT;
    }
    let ns = match clock_id {
        CLOCK_MONOTONIC => crate::clock::monotonic_ns(),
        CLOCK_REALTIME => crate::clock::realtime_ns(),
        _ => return EINVAL,
    };
    core::ptr::write(ts_ptr as *mut Timespec, ns_to_timespec(ns));
    0
}

/// SYS_NANOSLEEP: `*const Timespec req,*mut Timespec rem -> 0 | -errno`.
/// Relative sleep against CLOCK_MONOTONIC. `rem` is written only on early wake
/// (EINTR); signals are deferred in this kernel, so the sleep runs to completion
/// and `rem` is left untouched.
pub unsafe fn sys_nanosleep(req_ptr: u64, rem_ptr: u64) -> u64 {
    if !validate_user_buf(req_ptr, core::mem::size_of::<Timespec>() as u64) {
        return EFAULT;
    }
    if rem_ptr != 0 && !validate_user_buf(rem_ptr, core::mem::size_of::<Timespec>() as u64) {
        return EFAULT;
    }

    let req = core::ptr::read(req_ptr as *const Timespec);
    if req.tv_sec < 0 || req.tv_nsec < 0 || req.tv_nsec >= NANOS_PER_SEC as i64 {
        return EINVAL;
    }

    let total_ns = (req.tv_sec as u64)
        .saturating_mul(NANOS_PER_SEC)
        .saturating_add(req.tv_nsec as u64);
    if total_ns == 0 {
        return 0;
    }
    // Uncalibrated TSC ⇒ no usable deadline source; best-effort no-op rather than
    // parking on a deadline that never arrives (matches SYS_SLEEP).
    if crate::schedular::tsc_frequency() == 0 {
        return 0;
    }

    let deadline = crate::clock::tsc_deadline_in_ns(total_ns);
    crate::schedular::block_sleep(deadline)
}
