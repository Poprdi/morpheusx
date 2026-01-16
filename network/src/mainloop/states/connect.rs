//! TCP connection state — establishes TCP connection to server.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp::{Socket as TcpSocket, State as TcpState};
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpEndpoint};

use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};

use super::{FailedState, HttpState};

/// TCP connection state.
pub struct ConnectState {
    start_tsc: u64,
    connect_started: bool,
    target: Option<IpEndpoint>,
}

impl ConnectState {
    pub fn new() -> Self {
        Self {
            start_tsc: 0,
            connect_started: false,
            target: None,
        }
    }

    pub fn with_endpoint(addr: IpAddress, port: u16) -> Self {
        Self {
            start_tsc: 0,
            connect_started: false,
            target: Some(IpEndpoint::new(addr, port)),
        }
    }
}

impl Default for ConnectState {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: NetworkDriver> State<D> for ConnectState {
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
            serial::println("[TCP] Starting connection...");
        }

        let elapsed_ticks = tsc.saturating_sub(self.start_tsc);
        let timeout_ticks = ctx.timeouts.tcp_connect();
        if elapsed_ticks > timeout_ticks {
            serial::println("[TCP] ERROR: Connection timeout");
            return (Box::new(FailedState::new("TCP timeout")), StepResult::Failed("TCP timeout"));
        }

        let endpoint = match self.target {
            Some(ep) => ep,
            None => {
                let ip = match ctx.resolved_ip {
                    Some(ip) => ip,
                    None => {
                        serial::println("[TCP] ERROR: No resolved IP");
                        return (Box::new(FailedState::new("no IP")), StepResult::Failed("no IP"));
                    }
                };
                IpEndpoint::new(ip, ctx.resolved_port)
            }
        };

        // Get TCP handle from context
        let tcp_handle = match ctx.tcp_handle {
            Some(h) => h,
            None => {
                serial::println("[TCP] ERROR: No TCP socket");
                return (Box::new(FailedState::new("no TCP socket")), StepResult::Failed("no socket"));
            }
        };

        let socket = sockets.get_mut::<TcpSocket>(tcp_handle);

        if !self.connect_started {
            serial::print("[TCP] Connecting to ");
            match endpoint.addr {
                IpAddress::Ipv4(ip) => serial::print_ipv4(&ip.0),
                _ => serial::print("(IPv6)"),
            }
            serial::print(":");
            serial::print_u32(endpoint.port as u32);
            serial::println("");

            let local_port = 49152 + ((tsc & 0xFFFF) as u16 % 16384);

            if socket.connect(iface.context(), endpoint, local_port).is_err() {
                serial::println("[TCP] ERROR: Connect failed");
                return (Box::new(FailedState::new("connect failed")), StepResult::Failed("connect"));
            }
            self.connect_started = true;
            return (self, StepResult::Continue);
        }

        match socket.state() {
            TcpState::Established => {
                serial::println("[TCP] Connected!");
                serial::println("[TCP] -> HTTP");
                return (Box::new(HttpState::new(tcp_handle)), StepResult::Transition);
            }
            TcpState::SynSent | TcpState::SynReceived => {}
            TcpState::Closed | TcpState::TimeWait => {
                serial::println("[TCP] ERROR: Connection closed/reset");
                return (Box::new(FailedState::new("connection closed")), StepResult::Failed("closed"));
            }
            _ => {}
        }

        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "Connect"
    }
}
