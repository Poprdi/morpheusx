//! Standalone ICMP Ping Utility
//!
//! A minimal, no_std ICMP ping implementation for connectivity testing.
//! Designed for use in firmware/bootloader environments.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Ping Utility Structure                       │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                 │
//! │  ┌────────────┐  ┌────────────┐  ┌────────────┐                │
//! │  │   Types    │  │   Packet   │  │   Pinger   │                │
//! │  │            │  │  Builder   │  │   State    │                │
//! │  │ Ipv4Addr   │  │            │  │            │                │
//! │  │ MacAddress │  │ build_req  │  │ sequence   │                │
//! │  │ PingConfig │  │ parse_rep  │  │ stats      │                │
//! │  └────────────┘  └────────────┘  └────────────┘                │
//! │                                                                 │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_ping::{Pinger, PingConfig, Ipv4Addr};
//!
//! let mut pinger = Pinger::new(PingConfig::default());
//! let mut buffer = [0u8; 128];
//!
//! // Build ICMP echo request
//! let len = pinger.build_request(
//!     Ipv4Addr::new(192, 168, 1, 100),  // source
//!     Ipv4Addr::CLOUDFLARE_DNS,          // destination (1.1.1.1)
//!     &mut buffer,
//! )?;
//!
//! // ... send buffer[..len] via network driver ...
//! // ... receive reply into reply_buffer ...
//!
//! // Parse reply
//! let result = pinger.parse_reply(&reply_buffer)?;
//! if result.success {
//!     println!("Reply from {}: seq={}", result.target, result.sequence);
//! }
//! ```

#![no_std]
#![forbid(unsafe_code)]

mod types;
mod packet;
mod pinger;
mod checksum;

pub use types::{Ipv4Addr, MacAddress, PingConfig, PingResult, PingStats};
pub use packet::{IcmpType, ICMP_PROTOCOL};
pub use pinger::{Pinger, PingError};
pub use checksum::{calculate_checksum, verify_checksum};

/// Well-known ping targets
pub mod targets {
    use crate::Ipv4Addr;

    /// Cloudflare DNS - recommended, fastest response
    pub const CLOUDFLARE: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
    
    /// Google DNS - widely available
    pub const GOOGLE: Ipv4Addr = Ipv4Addr::new(8, 8, 8, 8);
    
    /// Quad9 DNS
    pub const QUAD9: Ipv4Addr = Ipv4Addr::new(9, 9, 9, 9);
    
    /// OpenDNS
    pub const OPENDNS: Ipv4Addr = Ipv4Addr::new(208, 67, 222, 222);
}
