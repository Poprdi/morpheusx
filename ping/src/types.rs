//! Core types: addresses, config, results, stats.

use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }

    /// Network byte order.
    pub const fn from_u32(addr: u32) -> Self {
        Self(addr.to_be_bytes())
    }

    /// Network byte order.
    pub const fn to_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub const UNSPECIFIED: Self = Self([0, 0, 0, 0]);
    pub const BROADCAST: Self = Self([255, 255, 255, 255]);
    pub const LOCALHOST: Self = Self([127, 0, 0, 1]);
    pub const CLOUDFLARE_DNS: Self = Self([1, 1, 1, 1]);
    pub const GOOGLE_DNS: Self = Self([8, 8, 8, 8]);

    pub const fn is_unspecified(&self) -> bool {
        self.0[0] == 0 && self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0
    }

    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    pub const fn octets(&self) -> [u8; 4] {
        self.0
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    pub const BROADCAST: Self = Self([0xFF; 6]);
    pub const ZERO: Self = Self([0x00; 6]);

    pub const fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PingConfig {
    pub timeout_ms: u32,
    pub count: u8,
    /// Payload bytes, excluding IP+ICMP headers.
    pub payload_size: u16,
    pub ttl: u8,
    pub interval_ms: u32,
}

impl Default for PingConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 3000,
            count: 3,
            payload_size: 56,
            ttl: 64,
            interval_ms: 1000,
        }
    }
}

impl PingConfig {
    /// Single ping, short timeout, small payload.
    pub const fn quick() -> Self {
        Self {
            timeout_ms: 2000,
            count: 1,
            payload_size: 32,
            ttl: 64,
            interval_ms: 0,
        }
    }

    pub const fn thorough() -> Self {
        Self {
            timeout_ms: 5000,
            count: 5,
            payload_size: 56,
            ttl: 64,
            interval_ms: 1000,
        }
    }

    /// Header-only echo.
    pub const fn minimal() -> Self {
        Self {
            timeout_ms: 1000,
            count: 1,
            payload_size: 0,
            ttl: 64,
            interval_ms: 0,
        }
    }

    /// IP + ICMP + payload.
    pub const fn packet_size(&self) -> usize {
        20 + 8 + self.payload_size as usize
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PingResult {
    pub target: Ipv4Addr,
    pub sequence: u16,
    pub rtt_ms: u32,
    pub reply_ttl: u8,
    pub success: bool,
}

impl PingResult {
    pub const fn success(target: Ipv4Addr, sequence: u16, rtt_ms: u32, reply_ttl: u8) -> Self {
        Self {
            target,
            sequence,
            rtt_ms,
            reply_ttl,
            success: true,
        }
    }

    pub const fn timeout(target: Ipv4Addr, sequence: u16) -> Self {
        Self {
            target,
            sequence,
            rtt_ms: 0,
            reply_ttl: 0,
            success: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PingStats {
    pub sent: u32,
    pub received: u32,
    pub lost: u32,
    pub min_rtt_ms: u32,
    pub max_rtt_ms: u32,
    rtt_sum_ms: u64,
}

impl PingStats {
    pub const fn new() -> Self {
        Self {
            sent: 0,
            received: 0,
            lost: 0,
            min_rtt_ms: u32::MAX,
            max_rtt_ms: 0,
            rtt_sum_ms: 0,
        }
    }

    pub fn record_sent(&mut self) {
        self.sent = self.sent.saturating_add(1);
    }

    pub fn record_reply(&mut self, rtt_ms: u32) {
        self.received = self.received.saturating_add(1);
        self.rtt_sum_ms = self.rtt_sum_ms.saturating_add(rtt_ms as u64);

        if rtt_ms < self.min_rtt_ms {
            self.min_rtt_ms = rtt_ms;
        }
        if rtt_ms > self.max_rtt_ms {
            self.max_rtt_ms = rtt_ms;
        }
    }

    pub fn record_lost(&mut self) {
        self.lost = self.lost.saturating_add(1);
    }

    pub const fn loss_percent(&self) -> u32 {
        if self.sent == 0 {
            100
        } else {
            (self.lost * 100) / self.sent
        }
    }

    pub const fn avg_rtt_ms(&self) -> u32 {
        if self.received == 0 {
            0
        } else {
            (self.rtt_sum_ms / self.received as u64) as u32
        }
    }

    pub const fn has_connectivity(&self) -> bool {
        self.received > 0
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipv4_addr() {
        let addr = Ipv4Addr::new(192, 168, 1, 1);
        assert_eq!(addr.octets(), [192, 168, 1, 1]);

        let addr2 = Ipv4Addr::from_u32(0xC0A80101);
        assert_eq!(addr, addr2);
    }

    #[test]
    fn test_ping_config() {
        let config = PingConfig::default();
        assert_eq!(config.packet_size(), 20 + 8 + 56);

        let quick = PingConfig::quick();
        assert_eq!(quick.count, 1);
    }

    #[test]
    fn test_ping_stats() {
        let mut stats = PingStats::new();

        stats.record_sent();
        stats.record_reply(10);
        stats.record_sent();
        stats.record_reply(20);
        stats.record_sent();
        stats.record_lost();

        assert_eq!(stats.sent, 3);
        assert_eq!(stats.received, 2);
        assert_eq!(stats.lost, 1);
        assert_eq!(stats.min_rtt_ms, 10);
        assert_eq!(stats.max_rtt_ms, 20);
        assert_eq!(stats.avg_rtt_ms(), 15);
        assert!(stats.loss_percent() > 0);
        assert!(stats.has_connectivity());
    }
}
