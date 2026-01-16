//! DNS resolution state — resolves hostname to IP address.
//!
//! For now, only handles direct IP addresses. DNS queries require
//! more complex smoltcp socket setup with static storage.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, Ipv4Address};

use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};

use super::{ConnectState, FailedState};

/// DNS resolution state.
pub struct DnsState {
    start_tsc: u64,
    hostname: Option<&'static str>,
}

impl DnsState {
    pub fn new() -> Self {
        Self {
            start_tsc: 0,
            hostname: None,
        }
    }

    pub fn with_hostname(hostname: &'static str) -> Self {
        Self {
            start_tsc: 0,
            hostname: Some(hostname),
        }
    }

    pub fn is_ip_address(s: &str) -> bool {
        parse_ipv4(s).is_some()
    }
}

impl Default for DnsState {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: NetworkDriver> State<D> for DnsState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        _iface: &mut Interface,
        _sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if self.start_tsc == 0 {
            self.start_tsc = tsc;
            serial::println("[DNS] Checking hostname...");
        }

        let hostname = self.hostname.unwrap_or(ctx.url_host);

        // Try to parse as IP address
        if let Some(ip) = parse_ipv4(hostname) {
            serial::print("[DNS] Host is IP: ");
            serial::print_ipv4(&ip.0);
            serial::println("");
            ctx.resolved_ip = Some(IpAddress::Ipv4(ip));
            serial::println("[DNS] -> Connect");
            return (Box::new(ConnectState::new()), StepResult::Transition);
        }

        // DNS lookup not implemented yet - require IP address for now
        serial::print("[DNS] ERROR: DNS not implemented, use IP address. Got: ");
        serial::println(hostname);
        (Box::new(FailedState::new("DNS not implemented")), StepResult::Failed("DNS not implemented"))
    }

    fn name(&self) -> &'static str {
        "DNS"
    }
}

/// Parse IPv4 address from dotted decimal string.
pub fn parse_ipv4(s: &str) -> Option<Ipv4Address> {
    let bytes = s.as_bytes();
    let mut octets = [0u8; 4];
    let mut octet_idx = 0;
    let mut current: u16 = 0;
    let mut digit_count = 0;

    for &b in bytes {
        if b == b'.' {
            if digit_count == 0 || current > 255 || octet_idx >= 3 {
                return None;
            }
            octets[octet_idx] = current as u8;
            octet_idx += 1;
            current = 0;
            digit_count = 0;
        } else if b.is_ascii_digit() {
            current = current * 10 + (b - b'0') as u16;
            digit_count += 1;
            if digit_count > 3 || current > 255 {
                return None;
            }
        } else {
            return None;
        }
    }

    if digit_count == 0 || current > 255 || octet_idx != 3 {
        return None;
    }
    octets[3] = current as u8;

    Some(Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]))
}
