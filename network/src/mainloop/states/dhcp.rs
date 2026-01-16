//! DHCP state — acquires IP address via DHCP.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::socket::dhcpv4::{Event as DhcpEvent, Socket as DhcpSocket};
use smoltcp::time::Instant;
use smoltcp::wire::IpCidr;

use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};

use super::{DnsState, FailedState};

/// DHCP acquisition state.
pub struct DhcpState {
    start_tsc: u64,
    got_ip: bool,
}

impl DhcpState {
    pub fn new() -> Self {
        Self {
            start_tsc: 0,
            got_ip: false,
        }
    }
}

impl Default for DhcpState {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: NetworkDriver> State<D> for DhcpState {
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
            serial::println("[DHCP] Starting DHCP discovery...");
        }

        let elapsed_ticks = tsc.saturating_sub(self.start_tsc);
        let timeout_ticks = ctx.timeouts.dhcp();
        if elapsed_ticks > timeout_ticks {
            serial::println("[DHCP] ERROR: Timeout");
            return (Box::new(FailedState::new("DHCP timeout")), StepResult::Failed("DHCP timeout"));
        }

        if self.got_ip {
            serial::println("[DHCP] -> DNS");
            return (Box::new(DnsState::new()), StepResult::Transition);
        }

        // Get DHCP handle from context
        let dhcp_handle = match ctx.dhcp_handle {
            Some(h) => h,
            None => {
                serial::println("[DHCP] ERROR: No DHCP socket");
                return (Box::new(FailedState::new("no DHCP socket")), StepResult::Failed("no socket"));
            }
        };

        let socket = sockets.get_mut::<DhcpSocket>(dhcp_handle);

        if let Some(event) = socket.poll() {
            match event {
                DhcpEvent::Configured(config) => {
                    let addr = config.address;
                    // Ipv4Address.0 is [u8; 4]
                    let ip_bytes = addr.address().0;
                    serial::print("[DHCP] Got IP: ");
                    serial::print_ipv4(&ip_bytes);
                    serial::print("/");
                    serial::print_u32(addr.prefix_len() as u32);
                    serial::println("");

                    iface.update_ip_addrs(|addrs| {
                        if let Some(addr_slot) = addrs.iter_mut().next() {
                            *addr_slot = IpCidr::Ipv4(addr);
                        }
                    });

                    if let Some(router) = config.router {
                        serial::print("[DHCP] Gateway: ");
                        serial::print_ipv4(&router.0);
                        serial::println("");
                        iface.routes_mut().add_default_ipv4_route(router).ok();
                    }

                    for (i, dns) in config.dns_servers.iter().enumerate() {
                        serial::print("[DHCP] DNS ");
                        serial::print_u32(i as u32);
                        serial::print(": ");
                        serial::print_ipv4(&dns.0);
                        serial::println("");
                    }

                    self.got_ip = true;
                }
                DhcpEvent::Deconfigured => {
                    serial::println("[DHCP] Deconfigured, retrying...");
                }
            }
        }

        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "DHCP"
    }
}
