//! Wait for PHY auto-neg; QEMU links instantly, real silicon doesn't.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;

use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};
use morpheus_nic::traits::NetworkDriver;

use super::{DhcpState, FailedState};

pub(crate) struct LinkWaitState {
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

    const LINK_TIMEOUT_SECS: u64 = 15;
    const STABILIZE_MS: u64 = 500;
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

        if self.link_established {
            let stabilize_ticks = (ctx.tsc_freq * Self::STABILIZE_MS) / 1000;
            if tsc.wrapping_sub(self.stable_start_tsc) >= stabilize_ticks {
                serial::println("[OK] Link stable");
                serial::println("[LINK] -> DHCP");
                return (Box::new(DhcpState::new()), StepResult::Transition);
            }
            return (self, StepResult::Continue);
        }

        if adapter.driver_link_up() {
            serial::println("");
            serial::println("[OK] PHY link established");
            serial::println("[NET] Link stabilization delay...");
            self.link_established = true;
            self.stable_start_tsc = tsc;
            return (self, StepResult::Continue);
        }

        let dot_ticks = ctx.tsc_freq * Self::DOT_INTERVAL_SECS;
        if tsc.wrapping_sub(self.last_dot_tsc) >= dot_ticks {
            serial::print(".");
            self.last_dot_tsc = tsc;
        }

        let timeout_ticks = ctx.tsc_freq * Self::LINK_TIMEOUT_SECS;
        if tsc.wrapping_sub(self.start_tsc) >= timeout_ticks {
            serial::println("");
            serial::println("[WARN] PHY link timeout - continuing anyway...");
            // Let DHCP surface the real error.
            serial::println("[LINK] -> DHCP");
            return (Box::new(DhcpState::new()), StepResult::Transition);
        }

        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "LinkWait"
    }
}
