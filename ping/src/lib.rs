//! no_std ICMP echo for firmware/bootloader connectivity checks.

#![no_std]
#![forbid(unsafe_code)]

mod checksum;
mod packet;
mod pinger;
mod types;

pub use checksum::{calculate_checksum, verify_checksum};
pub use packet::{IcmpType, ICMP_PROTOCOL};
pub use pinger::{PingError, Pinger};
pub use types::{Ipv4Addr, MacAddress, PingConfig, PingResult, PingStats};

pub mod targets {
    use crate::Ipv4Addr;

    pub const CLOUDFLARE: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
    pub const GOOGLE: Ipv4Addr = Ipv4Addr::new(8, 8, 8, 8);
    pub const QUAD9: Ipv4Addr = Ipv4Addr::new(9, 9, 9, 9);
    pub const OPENDNS: Ipv4Addr = Ipv4Addr::new(208, 67, 222, 222);
}
