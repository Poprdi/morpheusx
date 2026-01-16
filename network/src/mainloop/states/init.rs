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

use super::GptPrepState;

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
            serial::println("[INIT] -> GPT Prep");
            return (Box::new(GptPrepState::new()), StepResult::Transition);
        }

        serial::println("=====================================");
        serial::println("  MorpheusX Network State Machine");
        serial::println("=====================================");
        serial::println("");

        // Parse URL by computing indices first, avoiding borrow conflicts.
        // URL format: scheme://host[:port]/path
        let (scheme_end, default_port) = {
            let url = ctx.config.url;
            if url.starts_with("https://") {
                (8usize, 443u16)
            } else if url.starts_with("http://") {
                (7usize, 80u16)
            } else {
                serial::println("[INIT] ERROR: URL must start with http:// or https://");
                return (Box::new(super::FailedState::new("invalid URL scheme")), StepResult::Failed("invalid URL"));
            }
        };

        // Compute indices for host and path
        let url = ctx.config.url;
        let rest = &url[scheme_end..];
        
        let (host_end, path_start) = match rest.find('/') {
            Some(idx) => (scheme_end + idx, scheme_end + idx),
            None => (url.len(), url.len()),
        };

        let host_port_slice = &url[scheme_end..host_end];
        let (host_slice_end, port) = match host_port_slice.find(':') {
            Some(colon_idx) => {
                let port_str = &host_port_slice[colon_idx + 1..];
                let port = parse_port(port_str).unwrap_or(default_port);
                (scheme_end + colon_idx, port)
            }
            None => (host_end, default_port),
        };

        // Assign using direct indexing
        ctx.resolved_port = port;
        ctx.url_host = &ctx.config.url[scheme_end..host_slice_end];
        ctx.url_path = if path_start < ctx.config.url.len() {
            &ctx.config.url[path_start..]
        } else {
            "/"
        };

        serial::print("[INIT] URL: ");
        serial::println(ctx.config.url);
        serial::print("[INIT] Host: ");
        serial::println(ctx.url_host);
        serial::print("[INIT] Port: ");
        serial::print_u32(ctx.resolved_port as u32);
        serial::println("");
        serial::print("[INIT] Path: ");
        serial::println(ctx.url_path);
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
