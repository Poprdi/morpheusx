//! Initialization state — validates handoff, parses URL.

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

impl<D: NetworkDriver> State<D> for InitState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        iface: &mut Interface,
        sockets: &mut SocketSet<'_>,
        adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        _tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if self.validated {
            // Already validated, transition to DHCP
            serial::println("[INIT] -> DHCP");
            return (Box::new(DhcpState::new()), StepResult::Transition);
        }

        serial::println("=====================================");
        serial::println("  MorpheusX Network State Machine");
        serial::println("=====================================");
        serial::println("");

        // Parse URL
        let url = ctx.config.url;
        serial::print("[INIT] URL: ");
        serial::println(url);

        // Extract host and path from URL
        // Format: http://host[:port]/path or https://host[:port]/path
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

        // Find path separator
        let (host_port, path) = match url_without_scheme.find('/') {
            Some(idx) => (&url_without_scheme[..idx], &url_without_scheme[idx..]),
            None => (url_without_scheme, "/"),
        };

        // Check for port in host
        let host = match host_port.find(':') {
            Some(idx) => {
                if let Some(port) = parse_port(&host_port[idx + 1..]) {
                    ctx.resolved_port = port;
                }
                &host_port[..idx]
            }
            None => host_port,
        };

        // Store parsed values (these are string slices into the config)
        // For now just log them — real implementation would store differently
        serial::print("[INIT] Host: ");
        serial::println(host);
        serial::print("[INIT] Port: ");
        serial::print_u32(ctx.resolved_port as u32);
        serial::println("");
        serial::print("[INIT] Path: ");
        serial::println(path);

        // Validate TSC
        serial::print("[INIT] TSC freq: ");
        serial::print_hex(ctx.tsc_freq);
        serial::println(" Hz");

        if ctx.tsc_freq < 1_000_000_000 {
            serial::println("[INIT] WARNING: TSC freq seems low");
        }

        self.validated = true;
        
        // Stay in this state, next step will transition
        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "Init"
    }
}

/// Parse port number from string.
fn parse_port(s: &str) -> Option<u16> {
    let mut result: u32 = 0;
    for c in s.bytes() {
        if c < b'0' || c > b'9' {
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
