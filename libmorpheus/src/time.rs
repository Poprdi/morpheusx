//! Time — monotonic clock, Duration, and Instant.

use core::fmt;
use core::ops::{Add, AddAssign, Sub, SubAssign};

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

// ═══════════════════════════════════════════════════════════════════════
// Duration
// ═══════════════════════════════════════════════════════════════════════

/// A span of time, measured in nanoseconds.
///
/// 64-bit nanoseconds → max ~584 years.  Plenty.
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

    /// Fractional nanoseconds (the sub-second part).
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

// ═══════════════════════════════════════════════════════════════════════
// Instant
// ═══════════════════════════════════════════════════════════════════════

/// A measurement of a monotonically increasing clock.
///
/// Useful for measuring elapsed time.
///
/// # Example
/// ```ignore
/// let start = Instant::now();
/// do_work();
/// let elapsed = start.elapsed();
/// println!("took {:?}", elapsed);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant {
    nanos: u64,
}

impl Instant {
    /// Capture the current instant.
    pub fn now() -> Self {
        Self {
            nanos: clock_gettime(),
        }
    }

    /// Duration elapsed since this instant.
    pub fn elapsed(&self) -> Duration {
        let now = clock_gettime();
        Duration::from_nanos(now.saturating_sub(self.nanos))
    }

    /// Duration between `self` and a later instant.
    pub fn duration_since(&self, earlier: Instant) -> Duration {
        Duration::from_nanos(self.nanos.saturating_sub(earlier.nanos))
    }

    /// Checked addition.
    pub fn checked_add(&self, duration: Duration) -> Option<Self> {
        self.nanos
            .checked_add(duration.as_nanos())
            .map(|nanos| Self { nanos })
    }

    /// Checked subtraction.
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

// ═══════════════════════════════════════════════════════════════════════
// sleep with Duration
// ═══════════════════════════════════════════════════════════════════════

/// Sleep for a [`Duration`].
pub fn sleep(duration: Duration) {
    let ms = duration.as_millis();
    if ms > 0 {
        crate::process::sleep(ms);
    }
}
