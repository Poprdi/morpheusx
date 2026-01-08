//! Pinger - High-level ping interface

use crate::types::{Ipv4Addr, PingConfig, PingResult, PingStats};
use crate::packet::{
    build_ip_header, build_icmp_echo_request, parse_icmp_reply,
    IcmpType, ICMP_PROTOCOL, IP_HEADER_SIZE, ICMP_HEADER_SIZE, MIN_PACKET_SIZE,
};
use core::fmt;

/// Ping error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PingError {
    /// Buffer too small for packet
    BufferTooSmall,
    /// Invalid packet received
    InvalidPacket,
    /// Not an ICMP packet
    NotIcmp,
    /// Not an echo reply
    NotEchoReply,
    /// ID mismatch
    IdMismatch,
    /// Sequence mismatch
    SequenceMismatch,
    /// Destination unreachable
    DestUnreachable,
    /// Network unreachable
    NetworkUnreachable,
    /// Host unreachable
    HostUnreachable,
    /// Timeout
    Timeout,
}

impl fmt::Display for PingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooSmall => write!(f, "buffer too small"),
            Self::InvalidPacket => write!(f, "invalid packet"),
            Self::NotIcmp => write!(f, "not an ICMP packet"),
            Self::NotEchoReply => write!(f, "not an echo reply"),
            Self::IdMismatch => write!(f, "ID mismatch"),
            Self::SequenceMismatch => write!(f, "sequence mismatch"),
            Self::DestUnreachable => write!(f, "destination unreachable"),
            Self::NetworkUnreachable => write!(f, "network unreachable"),
            Self::HostUnreachable => write!(f, "host unreachable"),
            Self::Timeout => write!(f, "timeout"),
        }
    }
}

/// Result type for ping operations
pub type PingResultType<T> = Result<T, PingError>;

/// Stateful pinger for sending/receiving ICMP echo requests
#[derive(Debug)]
pub struct Pinger {
    /// Configuration
    config: PingConfig,
    /// Echo identifier (unique per pinger instance)
    id: u16,
    /// Current sequence number
    sequence: u16,
    /// Statistics
    stats: PingStats,
}

impl Pinger {
    /// Create a new pinger with the given configuration
    pub fn new(config: PingConfig) -> Self {
        Self {
            config,
            id: generate_id(),
            sequence: 0,
            stats: PingStats::new(),
        }
    }

    /// Create a pinger with specific ID (useful for testing)
    pub const fn with_id(config: PingConfig, id: u16) -> Self {
        Self {
            config,
            id,
            sequence: 0,
            stats: PingStats::new(),
        }
    }

    /// Get current configuration
    pub const fn config(&self) -> &PingConfig {
        &self.config
    }

    /// Get echo identifier
    pub const fn id(&self) -> u16 {
        self.id
    }

    /// Get current sequence number
    pub const fn sequence(&self) -> u16 {
        self.sequence
    }

    /// Get statistics
    pub const fn stats(&self) -> &PingStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats.reset();
    }

    /// Reset sequence number
    pub fn reset_sequence(&mut self) {
        self.sequence = 0;
    }

    /// Build an ICMP echo request packet
    ///
    /// # Arguments
    /// * `src` - Source IP address
    /// * `dst` - Destination IP address  
    /// * `buffer` - Output buffer (must be large enough for full packet)
    ///
    /// # Returns
    /// Number of bytes written to buffer, or error
    pub fn build_request(
        &mut self,
        src: Ipv4Addr,
        dst: Ipv4Addr,
        buffer: &mut [u8],
    ) -> PingResultType<usize> {
        let payload_size = self.config.payload_size as usize;
        let total_size = IP_HEADER_SIZE + ICMP_HEADER_SIZE + payload_size;

        if buffer.len() < total_size {
            return Err(PingError::BufferTooSmall);
        }

        // Build IP header
        let ip_len = build_ip_header(
            buffer,
            src,
            dst,
            total_size as u16,
            self.config.ttl,
            ICMP_PROTOCOL,
            self.id,
        );

        if ip_len != IP_HEADER_SIZE {
            return Err(PingError::BufferTooSmall);
        }

        // Build ICMP echo request
        let icmp_len = build_icmp_echo_request(
            &mut buffer[IP_HEADER_SIZE..],
            self.id,
            self.sequence,
            payload_size,
        );

        if icmp_len != ICMP_HEADER_SIZE + payload_size {
            return Err(PingError::BufferTooSmall);
        }

        // Increment sequence for next request
        self.sequence = self.sequence.wrapping_add(1);
        
        // Record sent
        self.stats.record_sent();

        Ok(total_size)
    }

    /// Parse an ICMP echo reply packet
    ///
    /// # Arguments
    /// * `data` - Received packet data (including IP header)
    /// * `expected_seq` - Expected sequence number (None to accept any)
    ///
    /// # Returns
    /// Parsed ping result or error
    pub fn parse_reply(
        &mut self,
        data: &[u8],
        expected_seq: Option<u16>,
    ) -> PingResultType<PingResult> {
        if data.len() < MIN_PACKET_SIZE {
            return Err(PingError::InvalidPacket);
        }

        let parsed = parse_icmp_reply(data).ok_or(PingError::InvalidPacket)?;

        // Check for ICMP errors
        if parsed.icmp_type == IcmpType::DestUnreachable as u8 {
            return match parsed.icmp_code {
                0 => Err(PingError::NetworkUnreachable),
                1 => Err(PingError::HostUnreachable),
                _ => Err(PingError::DestUnreachable),
            };
        }

        // Must be echo reply
        if parsed.icmp_type != IcmpType::EchoReply as u8 {
            return Err(PingError::NotEchoReply);
        }

        // Verify ID matches
        if parsed.id != self.id {
            return Err(PingError::IdMismatch);
        }

        // Verify sequence if expected
        if let Some(seq) = expected_seq {
            if parsed.sequence != seq {
                return Err(PingError::SequenceMismatch);
            }
        }

        Ok(PingResult {
            target: parsed.src_ip,
            sequence: parsed.sequence,
            rtt_ms: 0, // Caller must calculate from timestamps
            reply_ttl: parsed.ttl,
            success: true,
        })
    }

    /// Record a successful reply with RTT
    pub fn record_success(&mut self, rtt_ms: u32) {
        self.stats.record_reply(rtt_ms);
    }

    /// Record a timeout
    pub fn record_timeout(&mut self) {
        self.stats.record_lost();
    }

    /// Check if we have any connectivity based on stats
    pub fn has_connectivity(&self) -> bool {
        self.stats.has_connectivity()
    }
}

impl Default for Pinger {
    fn default() -> Self {
        Self::new(PingConfig::default())
    }
}

/// Generate a pseudo-random ID based on some simple mixing
fn generate_id() -> u16 {
    // In real implementation, would use timestamp or RNG
    // For no_std, we use a simple counter-based approach
    static mut COUNTER: u16 = 0;
    
    // SAFETY: Single-threaded bootloader context
    unsafe {
        COUNTER = COUNTER.wrapping_add(1);
        COUNTER.wrapping_mul(31421).wrapping_add(6927)
    }
}

/// Quick ping check - build request, returns buffer ready to send
pub fn quick_ping_request(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    buffer: &mut [u8],
) -> PingResultType<(usize, u16)> {
    let mut pinger = Pinger::new(PingConfig::quick());
    let len = pinger.build_request(src, dst, buffer)?;
    Ok((len, pinger.id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pinger_new() {
        let pinger = Pinger::new(PingConfig::default());
        assert_eq!(pinger.sequence(), 0);
        assert!(!pinger.has_connectivity());
    }

    #[test]
    fn test_build_request() {
        let mut pinger = Pinger::with_id(PingConfig::quick(), 0x1234);
        let src = Ipv4Addr::new(192, 168, 1, 100);
        let dst = Ipv4Addr::CLOUDFLARE_DNS;
        let mut buffer = [0u8; 128];

        let len = pinger.build_request(src, dst, &mut buffer).unwrap();
        
        // IP(20) + ICMP(8) + payload(32 for quick config)
        assert_eq!(len, 20 + 8 + 32);
        assert_eq!(pinger.sequence(), 1); // Should increment
        assert_eq!(pinger.stats().sent, 1);
    }

    #[test]
    fn test_build_request_buffer_too_small() {
        let mut pinger = Pinger::new(PingConfig::default());
        let src = Ipv4Addr::new(192, 168, 1, 100);
        let dst = Ipv4Addr::CLOUDFLARE_DNS;
        let mut buffer = [0u8; 10]; // Too small

        let result = pinger.build_request(src, dst, &mut buffer);
        assert_eq!(result, Err(PingError::BufferTooSmall));
    }

    #[test]
    fn test_sequence_increment() {
        let mut pinger = Pinger::with_id(PingConfig::quick(), 0x1234);
        let src = Ipv4Addr::new(192, 168, 1, 100);
        let dst = Ipv4Addr::CLOUDFLARE_DNS;
        let mut buffer = [0u8; 128];

        assert_eq!(pinger.sequence(), 0);
        
        pinger.build_request(src, dst, &mut buffer).unwrap();
        assert_eq!(pinger.sequence(), 1);
        
        pinger.build_request(src, dst, &mut buffer).unwrap();
        assert_eq!(pinger.sequence(), 2);
    }

    #[test]
    fn test_stats_tracking() {
        let mut pinger = Pinger::new(PingConfig::default());
        
        pinger.record_success(10);
        pinger.record_success(20);
        pinger.record_timeout();
        
        assert_eq!(pinger.stats().received, 2);
        assert_eq!(pinger.stats().lost, 1);
        assert!(pinger.has_connectivity());
    }

    #[test]
    fn test_quick_ping_request() {
        let src = Ipv4Addr::new(192, 168, 1, 100);
        let dst = Ipv4Addr::CLOUDFLARE_DNS;
        let mut buffer = [0u8; 128];

        let (len, id) = quick_ping_request(src, dst, &mut buffer).unwrap();
        
        assert!(len > 0);
        assert!(id != 0);
    }
}
