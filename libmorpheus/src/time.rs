//! Monotonic clock, Duration, Instant.

use core::fmt;
use core::ops::{Add, AddAssign, Sub, SubAssign};

use crate::raw::*;
use morpheus_foundation::flags::{CLOCK_MONOTONIC, CLOCK_REALTIME};
use morpheus_foundation::types::Timespec;

/// Monotonic nanoseconds since boot. Derived from TSC; returns 0 if TSC uncalibrated.
pub fn clock_gettime() -> u64 {
    unsafe { syscall0(SYS_CLOCK) }
}

const NANOS_PER_SEC: u64 = 1_000_000_000;

/// Read a clock as ns via SYS_CLOCK_GETTIME. Returns 0 on error (only an invalid
/// clock id, which we never pass).
fn clock_gettime_ns(clock_id: u64) -> u64 {
    let mut ts = Timespec::default();
    let r = unsafe { sys_clock_gettime(clock_id, &mut ts as *mut Timespec as u64) };
    if morpheus_foundation::errno::is_error(r) {
        return 0;
    }
    (ts.tv_sec as u64)
        .wrapping_mul(NANOS_PER_SEC)
        .wrapping_add(ts.tv_nsec as u64)
}

pub fn uptime_ms() -> u64 {
    clock_gettime() / 1_000_000
}

pub fn uptime_us() -> u64 {
    clock_gettime() / 1_000
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration {
    nanos: u64,
}

impl Duration {
    pub const ZERO: Duration = Duration { nanos: 0 };

    pub const fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
    }

    pub const fn from_micros(micros: u64) -> Self {
        Self {
            nanos: micros * 1_000,
        }
    }

    pub const fn from_millis(millis: u64) -> Self {
        Self {
            nanos: millis * 1_000_000,
        }
    }

    pub const fn from_secs(secs: u64) -> Self {
        Self {
            nanos: secs * 1_000_000_000,
        }
    }

    pub const fn as_nanos(&self) -> u64 {
        self.nanos
    }

    pub const fn as_micros(&self) -> u64 {
        self.nanos / 1_000
    }

    pub const fn as_millis(&self) -> u64 {
        self.nanos / 1_000_000
    }

    pub const fn as_secs(&self) -> u64 {
        self.nanos / 1_000_000_000
    }

    pub const fn subsec_nanos(&self) -> u32 {
        (self.nanos % 1_000_000_000) as u32
    }

    pub const fn is_zero(&self) -> bool {
        self.nanos == 0
    }

    pub const fn saturating_add(self, rhs: Self) -> Self {
        Self {
            nanos: self.nanos.saturating_add(rhs.nanos),
        }
    }

    pub const fn saturating_sub(self, rhs: Self) -> Self {
        Self {
            nanos: self.nanos.saturating_sub(rhs.nanos),
        }
    }

    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.nanos.checked_add(rhs.nanos) {
            Some(n) => Some(Self { nanos: n }),
            None => None,
        }
    }

    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.nanos.checked_sub(rhs.nanos) {
            Some(n) => Some(Self { nanos: n }),
            None => None,
        }
    }
}

impl Add for Duration {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            nanos: self.nanos + rhs.nanos,
        }
    }
}

impl AddAssign for Duration {
    fn add_assign(&mut self, rhs: Self) {
        self.nanos += rhs.nanos;
    }
}

impl Sub for Duration {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            nanos: self.nanos - rhs.nanos,
        }
    }
}

impl SubAssign for Duration {
    fn sub_assign(&mut self, rhs: Self) {
        self.nanos -= rhs.nanos;
    }
}

impl fmt::Debug for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.as_secs();
        let subsec = self.subsec_nanos();
        if secs > 0 {
            write!(f, "{}.{:09}s", secs, subsec)
        } else if subsec >= 1_000_000 {
            write!(f, "{}ms", self.as_millis())
        } else if subsec >= 1_000 {
            write!(f, "{}us", self.as_micros())
        } else {
            write!(f, "{}ns", self.nanos)
        }
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant {
    nanos: u64,
}

impl Instant {
    pub fn now() -> Self {
        Self {
            nanos: clock_gettime_ns(CLOCK_MONOTONIC),
        }
    }

    pub fn elapsed(&self) -> Duration {
        let now = clock_gettime_ns(CLOCK_MONOTONIC);
        Duration::from_nanos(now.saturating_sub(self.nanos))
    }

    pub fn duration_since(&self, earlier: Instant) -> Duration {
        Duration::from_nanos(self.nanos.saturating_sub(earlier.nanos))
    }

    pub fn checked_add(&self, duration: Duration) -> Option<Self> {
        self.nanos
            .checked_add(duration.as_nanos())
            .map(|nanos| Self { nanos })
    }

    pub fn checked_sub(&self, duration: Duration) -> Option<Self> {
        self.nanos
            .checked_sub(duration.as_nanos())
            .map(|nanos| Self { nanos })
    }
}

impl Add<Duration> for Instant {
    type Output = Self;
    fn add(self, rhs: Duration) -> Self {
        Self {
            nanos: self.nanos + rhs.as_nanos(),
        }
    }
}

impl Sub<Duration> for Instant {
    type Output = Self;
    fn sub(self, rhs: Duration) -> Self {
        Self {
            nanos: self.nanos - rhs.as_nanos(),
        }
    }
}

impl Sub for Instant {
    type Output = Duration;
    fn sub(self, rhs: Self) -> Duration {
        Duration::from_nanos(self.nanos.saturating_sub(rhs.nanos))
    }
}

impl fmt::Debug for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Instant({}ns)", self.nanos)
    }
}

/// Relative sleep against the monotonic clock with full ns precision (backs
/// `std::thread::sleep`). Signals are deferred kernel-side, so this never returns
/// early; the `rem` slot is unused.
pub fn sleep(duration: Duration) {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        return;
    }
    let req = Timespec {
        tv_sec: (nanos / NANOS_PER_SEC) as i64,
        tv_nsec: (nanos % NANOS_PER_SEC) as i64,
    };
    unsafe {
        sys_nanosleep(&req as *const Timespec as u64, 0);
    }
}

/// Wall-clock time (`std::time::SystemTime`), CLOCK_REALTIME nanoseconds since the
/// Unix epoch. Anchored to the boot RTC read; absent an RTC it reads 1970+uptime.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SystemTime {
    nanos: u64,
}

impl SystemTime {
    pub const UNIX_EPOCH: SystemTime = SystemTime { nanos: 0 };

    pub fn now() -> Self {
        Self {
            nanos: clock_gettime_ns(CLOCK_REALTIME),
        }
    }

    /// `Ok(elapsed)` if `self >= earlier`, else `Err(how_far_self_is_before)` —
    /// the shape `std::time::SystemTimeError` carries.
    pub fn duration_since(&self, earlier: SystemTime) -> Result<Duration, Duration> {
        if self.nanos >= earlier.nanos {
            Ok(Duration::from_nanos(self.nanos - earlier.nanos))
        } else {
            Err(Duration::from_nanos(earlier.nanos - self.nanos))
        }
    }

    pub fn elapsed(&self) -> Result<Duration, Duration> {
        SystemTime::now().duration_since(*self)
    }

    pub fn checked_add(&self, duration: Duration) -> Option<Self> {
        self.nanos
            .checked_add(duration.as_nanos())
            .map(|nanos| Self { nanos })
    }

    pub fn checked_sub(&self, duration: Duration) -> Option<Self> {
        self.nanos
            .checked_sub(duration.as_nanos())
            .map(|nanos| Self { nanos })
    }
}

impl Add<Duration> for SystemTime {
    type Output = Self;
    fn add(self, rhs: Duration) -> Self {
        Self {
            nanos: self.nanos + rhs.as_nanos(),
        }
    }
}

impl Sub<Duration> for SystemTime {
    type Output = Self;
    fn sub(self, rhs: Duration) -> Self {
        Self {
            nanos: self.nanos - rhs.as_nanos(),
        }
    }
}

impl fmt::Debug for SystemTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SystemTime({}ns since epoch)", self.nanos)
    }
}
