//! Time and timing module.
//!
//! TSC-based timing with calibrated timeouts.

/// Timeout configuration derived from TSC frequency.
#[derive(Debug, Clone, Copy)]
pub struct TimeoutConfig {
    ticks_per_ms: u64,
}

impl TimeoutConfig {
    /// Create from TSC frequency.
    pub fn new(tsc_freq: u64) -> Self {
        Self {
            ticks_per_ms: tsc_freq / 1_000,
        }
    }

    /// DHCP timeout (30 seconds)
    #[inline]
    pub fn dhcp(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// DNS query timeout (5 seconds)
    #[inline]
    pub fn dns(&self) -> u64 {
        5_000 * self.ticks_per_ms
    }

    /// TCP connect timeout (30 seconds)
    #[inline]
    pub fn tcp_connect(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// TCP close timeout (10 seconds)
    #[inline]
    pub fn tcp_close(&self) -> u64 {
        10_000 * self.ticks_per_ms
    }

    /// HTTP send timeout (30 seconds)
    #[inline]
    pub fn http_send(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// HTTP receive timeout (60 seconds)
    #[inline]
    pub fn http_receive(&self) -> u64 {
        60_000 * self.ticks_per_ms
    }

    /// HTTP idle timeout between chunks (30 seconds)
    #[inline]
    pub fn http_idle(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// Main loop iteration warning threshold (5ms)
    #[inline]
    pub fn loop_warning(&self) -> u64 {
        5 * self.ticks_per_ms
    }

    /// Device reset timeout (100ms)
    #[inline]
    pub fn device_reset(&self) -> u64 {
        100 * self.ticks_per_ms
    }

    /// Convert milliseconds to ticks
    #[inline]
    pub fn ms_to_ticks(&self, ms: u64) -> u64 {
        ms * self.ticks_per_ms
    }

    /// Convert ticks to milliseconds
    #[inline]
    pub fn ticks_to_ms(&self, ticks: u64) -> u64 {
        ticks / self.ticks_per_ms
    }
}
