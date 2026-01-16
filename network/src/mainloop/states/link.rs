//! PHY link wait state — waits for Ethernet link to establish.
//!
//! Real hardware (unlike QEMU) needs time for PHY auto-negotiation.
//! This state polls the driver until link_up() returns true, with
//! timeout handling and a brief stabilization delay.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;

use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};

use super::{DhcpState, FailedState};

/// PHY link wait state.
pub struct LinkWaitState {
    started: bool,
    start_tsc: u64,
    link_established: bool,
    stable_start_tsc: u64,
    last_dot_tsc: u64,
}

impl LinkWaitState {
    pub fn new() -> Self {
        Self {
            started: false,
            start_tsc: 0,
            link_established: false,
            stable_start_tsc: 0,
            last_dot_tsc: 0,
        }
    }

    /// 15 second timeout for PHY auto-negotiation.
    const LINK_TIMEOUT_SECS: u64 = 15;

    /// 500ms stabilization delay after link comes up.
    const STABILIZE_MS: u64 = 500;

    /// Print progress dot every second.
    const DOT_INTERVAL_SECS: u64 = 1;
}

impl Default for LinkWaitState {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: NetworkDriver> State<D> for LinkWaitState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        _iface: &mut Interface,
        _sockets: &mut SocketSet<'_>,
        adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if !self.started {
            self.started = true;
            self.start_tsc = tsc;
            self.last_dot_tsc = tsc;
            serial::println("[NET] Waiting for PHY link...");
        }

        // If link already established, wait for stabilization
        if self.link_established {
            let stabilize_ticks = (ctx.tsc_freq * Self::STABILIZE_MS) / 1000;
            if tsc.wrapping_sub(self.stable_start_tsc) >= stabilize_ticks {
                serial::println("[OK] Link stable");
                serial::println("[LINK] -> DHCP");
                return (Box::new(DhcpState::new()), StepResult::Transition);
            }
            // Still stabilizing
            return (self, StepResult::Continue);
        }

        // Check if link is up
        if adapter.driver_link_up() {
            serial::println("");
            serial::println("[OK] PHY link established");
            serial::println("[NET] Link stabilization delay...");
            self.link_established = true;
            self.stable_start_tsc = tsc;
            return (self, StepResult::Continue);
        }

        // Print progress dot every second
        let dot_ticks = ctx.tsc_freq * Self::DOT_INTERVAL_SECS;
        if tsc.wrapping_sub(self.last_dot_tsc) >= dot_ticks {
            serial::print(".");
            self.last_dot_tsc = tsc;
        }

        // Check timeout
        let timeout_ticks = ctx.tsc_freq * Self::LINK_TIMEOUT_SECS;
        if tsc.wrapping_sub(self.start_tsc) >= timeout_ticks {
            serial::println("");
            serial::println("[WARN] PHY link timeout - continuing anyway...");
            // Continue to DHCP even without link - it will fail with proper error
            // if link really isn't available
            serial::println("[LINK] -> DHCP");
            return (Box::new(DhcpState::new()), StepResult::Transition);
        }

        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "LinkWait"
    }
}
