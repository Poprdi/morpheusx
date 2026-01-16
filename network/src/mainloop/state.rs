//! State trait definition for the download state machine.
//!
//! Follows the State design pattern where each state is a separate type
//! implementing a common trait. State transitions consume `self` and
//! return a new boxed state.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;

use crate::driver::traits::NetworkDriver;
use super::adapter::SmoltcpAdapter;
use super::context::Context;

/// Result of a single state machine step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// Continue in current state
    Continue,
    /// Transition to a new state (returned by step())
    Transition,
    /// Operation complete — success
    Done,
    /// Operation failed
    Failed(&'static str),
}

/// The State trait — each download phase implements this.
///
/// The `self: Box<Self>` pattern allows states to consume themselves
/// and return a different state type, enabling type-safe transitions.
pub trait State<D: NetworkDriver> {
    /// Execute one step of this state.
    ///
    /// Returns the next state (which may be self for Continue,
    /// or a new state for Transition).
    fn step(
        self: Box<Self>,
        ctx: &mut Context<'_>,
        iface: &mut Interface,
        sockets: &mut SocketSet<'_>,
        adapter: &mut SmoltcpAdapter<'_, D>,
        now: Instant,
        tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult);

    /// Human-readable name for logging.
    fn name(&self) -> &'static str;

    /// Whether this is a terminal state.
    fn is_terminal(&self) -> bool {
        false
    }
}
