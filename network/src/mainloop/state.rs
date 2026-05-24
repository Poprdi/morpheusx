//! Download state machine trait. `self: Box<Self>` lets transitions return a new state type.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;

use super::adapter::SmoltcpAdapter;
use super::context::Context;
use crate::driver::traits::NetworkDriver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    Continue,
    Transition,
    Done,
    Failed(&'static str),
}

pub trait State<D: NetworkDriver> {
    fn step(
        self: Box<Self>,
        ctx: &mut Context<'_>,
        iface: &mut Interface,
        sockets: &mut SocketSet<'_>,
        adapter: &mut SmoltcpAdapter<'_, D>,
        now: Instant,
        tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult);

    fn name(&self) -> &'static str;

    fn is_terminal(&self) -> bool {
        false
    }
}
