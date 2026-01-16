//! Initialization state — parses URL.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;

use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};

use super::DhcpState;

/// Initialization state.
pub struct InitState {
    validated: bool,
}

impl InitState {
    pub fn new() -> Self {
        Self { validated: false }
    }
}

impl Default for InitState {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: NetworkDriver> State<D> for InitState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        _iface: &mut Interface,
        _sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        _tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if self.validated {
            serial::println("[INIT] -> DHCP");
            return (Box::new(DhcpState::new()), StepResult::Transition);
        }

        serial::println("=====================================");
        serial::println("  MorpheusX Network State Machine");
        serial::println("=====================================");
        serial::println("");

        let url = ctx.url;
        serial::print("[INIT] URL: ");
        serial::println(url);

        // Parse URL: http://host[:port]/path
        let url_without_scheme = if url.starts_with("https://") {
            ctx.resolved_port = 443;
            &url[8..]
        } else if url.starts_with("http://") {
            ctx.resolved_port = 80;
            &url[7..]
        } else {
            serial::println("[INIT] ERROR: URL must start with http:// or https://");
            return (Box::new(super::FailedState::new("invalid URL scheme")), StepResult::Failed("invalid URL"));
        };

        let (host_port, path) = match url_without_scheme.find('/') {
            Some(idx) => (&url_without_scheme[..idx], &url_without_scheme[idx..]),
            None => (url_without_scheme, "/"),
        };

        let host = match host_port.find(':') {
            Some(idx) => {
                if let Some(port) = parse_port(&host_port[idx + 1..]) {
                    ctx.resolved_port = port;
                }
                &host_port[..idx]
            }
            None => host_port,
        };

        // Store parsed URL parts in context
        ctx.url_host = host;
        ctx.url_path = path;

        serial::print("[INIT] Host: ");
        serial::println(host);
        serial::print("[INIT] Port: ");
        serial::print_u32(ctx.resolved_port as u32);
        serial::println("");
        serial::print("[INIT] Path: ");
        serial::println(path);
        serial::print("[INIT] TSC freq: ");
        serial::print_u32((ctx.tsc_freq / 1_000_000) as u32);
        serial::println(" MHz");

        self.validated = true;
        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "Init"
    }
}

fn parse_port(s: &str) -> Option<u16> {
    let mut result: u32 = 0;
    for c in s.bytes() {
        if !c.is_ascii_digit() {
            break;
        }
        result = result * 10 + (c - b'0') as u32;
        if result > 65535 {
            return None;
        }
    }
    if result > 0 {
        Some(result as u16)
    } else {
        None
    }
}
