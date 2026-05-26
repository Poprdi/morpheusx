//! TSC-derived timeouts. Returns tick counts; compare against `read_tsc()`.

#[derive(Debug, Clone, Copy)]
pub struct TimeoutConfig {
    ticks_per_ms: u64,
}

impl TimeoutConfig {
    pub fn new(tsc_freq: u64) -> Self {
        Self {
            ticks_per_ms: tsc_freq / 1_000,
        }
    }

    /// 30 s.
    #[inline]
    pub fn dhcp(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// 5 s.
    #[inline]
    pub fn dns(&self) -> u64 {
        5_000 * self.ticks_per_ms
    }

    /// 30 s.
    #[inline]
    pub fn tcp_connect(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// 10 s.
    #[inline]
    pub fn tcp_close(&self) -> u64 {
        10_000 * self.ticks_per_ms
    }

    /// 30 s.
    #[inline]
    pub fn http_send(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// 60 s.
    #[inline]
    pub fn http_receive(&self) -> u64 {
        60_000 * self.ticks_per_ms
    }

    /// 30 s between chunks.
    #[inline]
    pub fn http_idle(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// 5 ms main-loop iteration warning threshold.
    #[inline]
    pub fn loop_warning(&self) -> u64 {
        5 * self.ticks_per_ms
    }

    /// 100 ms.
    #[inline]
    pub fn device_reset(&self) -> u64 {
        100 * self.ticks_per_ms
    }

    #[inline]
    pub fn ms_to_ticks(&self, ms: u64) -> u64 {
        ms * self.ticks_per_ms
    }

    #[inline]
    pub fn ticks_to_ms(&self, ticks: u64) -> u64 {
        ticks / self.ticks_per_ms
    }
}
