//! Terminal states — success and failure endpoints.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;

use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};

/// Success terminal state.
pub struct DoneState {
    logged: bool,
}

impl DoneState {
    pub fn new() -> Self {
        Self { logged: false }
    }
}

impl Default for DoneState {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: NetworkDriver> State<D> for DoneState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        _iface: &mut Interface,
        _sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        _tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if !self.logged {
            serial::println("=================================");
            serial::println("        DOWNLOAD COMPLETE        ");
            serial::println("=================================");
            serial::print("Total bytes: ");
            serial::print_u32((ctx.bytes_downloaded / 1024) as u32);
            serial::println(" KB");
            self.logged = true;
        }

        (self, StepResult::Done)
    }

    fn name(&self) -> &'static str {
        "Done"
    }
}

/// Failure terminal state.
pub struct FailedState {
    reason: &'static str,
    logged: bool,
}

impl FailedState {
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason,
            logged: false,
        }
    }

    /// Get failure reason.
    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl<D: NetworkDriver> State<D> for FailedState {
    fn step(
        mut self: Box<Self>,
        _ctx: &mut Context<'_>,
        _iface: &mut Interface,
        _sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        _tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if !self.logged {
            serial::println("=================================");
            serial::println("        DOWNLOAD FAILED          ");
            serial::println("=================================");
            serial::print("Reason: ");
            serial::println(self.reason);
            self.logged = true;
        }

        (self, StepResult::Failed(self.reason))
    }

    fn name(&self) -> &'static str {
        "Failed"
    }
}
