//! DNS resolution state — resolves hostname to IP address.
//!
//! Uses smoltcp's DNS socket for resolution. Falls back to direct
//! IP address parsing when hostname is already an IP.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::socket::dns::{GetQueryResultError, QueryHandle, Socket as DnsSocket};
use smoltcp::time::Instant;
use smoltcp::wire::{DnsQueryType, IpAddress, Ipv4Address};

use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};

use super::{ConnectState, FailedState};

/// Static storage for DNS queries (smoltcp requirement).
static mut DNS_QUERIES: [Option<smoltcp::socket::dns::DnsQuery>; 1] = [None];

/// DNS resolution state.
pub struct DnsState {
    start_tsc: u64,
    query_handle: Option<QueryHandle>,
    dns_handle_added: bool,
}

impl DnsState {
    pub fn new() -> Self {
        Self {
            start_tsc: 0,
            query_handle: None,
            dns_handle_added: false,
        }
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
        iface: &mut Interface,
        sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if self.start_tsc == 0 {
            self.start_tsc = tsc;
            serial::println("[DNS] Starting resolution...");
        }

        // Check timeout
        let elapsed = tsc.saturating_sub(self.start_tsc);
        let timeout = ctx.timeouts.dns();
        if elapsed > timeout {
            serial::println("[DNS] ERROR: Timeout");
            return (Box::new(FailedState::new("DNS timeout")), StepResult::Failed("DNS timeout"));
        }

        let hostname = ctx.url_host;

        // Try parsing as IP address first
        if let Some(ip) = parse_ipv4(hostname) {
            serial::print("[DNS] Host is IP: ");
            serial::print_ipv4(&ip.0);
            serial::println("");
            ctx.resolved_ip = Some(IpAddress::Ipv4(ip));
            serial::println("[DNS] -> Connect");
            return (Box::new(ConnectState::new()), StepResult::Transition);
        }

        // Need DNS resolution
        serial::print("[DNS] Resolving: ");
        serial::println(hostname);

        // Get DNS server from DHCP
        let dns_server = match ctx.dns_servers.iter().find_map(|s| *s) {
            Some(IpAddress::Ipv4(ip)) => ip,
            _ => {
                serial::println("[DNS] ERROR: No DNS server from DHCP");
                return (Box::new(FailedState::new("no DNS server")), StepResult::Failed("no DNS"));
            }
        };

        // Create DNS socket if not done yet
        if !self.dns_handle_added {
            serial::print("[DNS] Using server: ");
            serial::print_ipv4(&dns_server.0);
            serial::println("");

            let dns_servers: &[IpAddress] = &[IpAddress::Ipv4(dns_server)];
            let dns_socket = unsafe { DnsSocket::new(dns_servers, &mut DNS_QUERIES[..]) };
            let handle = sockets.add(dns_socket);
            ctx.dns_handle = Some(handle);
            self.dns_handle_added = true;
        }

        let dns_handle = match ctx.dns_handle {
            Some(h) => h,
            None => {
                serial::println("[DNS] ERROR: No DNS socket");
                return (Box::new(FailedState::new("no DNS socket")), StepResult::Failed("no socket"));
            }
        };

        // Start query if not started
        if self.query_handle.is_none() {
            let dns = sockets.get_mut::<DnsSocket>(dns_handle);
            match dns.start_query(iface.context(), hostname, DnsQueryType::A) {
                Ok(handle) => {
                    serial::println("[DNS] Query sent");
                    self.query_handle = Some(handle);
                }
                Err(_) => {
                    serial::println("[DNS] ERROR: Query start failed");
                    return (Box::new(FailedState::new("DNS query failed")), StepResult::Failed("query"));
                }
            }
            return (self, StepResult::Continue);
        }

        // Poll for result
        let query_handle = self.query_handle.unwrap();
        let dns = sockets.get_mut::<DnsSocket>(dns_handle);
        match dns.get_query_result(query_handle) {
            Ok(addrs) => {
                // Find first IPv4 address
                for addr in addrs {
                    if let IpAddress::Ipv4(ip) = addr {
                        serial::print("[DNS] Resolved: ");
                        serial::print_ipv4(&ip.0);
                        serial::println("");
                        ctx.resolved_ip = Some(IpAddress::Ipv4(ip));
                        serial::println("[DNS] -> Connect");
                        return (Box::new(ConnectState::new()), StepResult::Transition);
                    }
                }
                serial::println("[DNS] ERROR: No IPv4 in response");
                (Box::new(FailedState::new("no IPv4")), StepResult::Failed("no IPv4"))
            }
            Err(GetQueryResultError::Pending) => {
                // Still waiting
                (self, StepResult::Continue)
            }
            Err(GetQueryResultError::Failed) => {
                serial::println("[DNS] ERROR: Query failed");
                (Box::new(FailedState::new("DNS failed")), StepResult::Failed("DNS failed"))
            }
        }
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
