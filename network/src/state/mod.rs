//! State machine module.
//!
//! Non-blocking state machines for all network operations.
//! Each state machine has a `step()` method that returns immediately.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5

pub mod dhcp;
pub mod tcp;
pub mod http;
pub mod download;

// TODO: Implement StepResult and StateMachine trait
//
// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
// pub enum StepResult {
//     Pending,
//     Done,
//     Timeout,
//     Failed,
// }
//
// pub trait StateMachine {
//     type Output;
//     type Error;
//     
//     fn step(
//         &mut self,
//         iface: &mut Interface,
//         sockets: &mut SocketSet,
//         now_tsc: u64,
//         timeouts: &TimeoutConfig,
//     ) -> StepResult;
//     
//     fn output(&self) -> &Self::Output;
//     fn error(&self) -> &Self::Error;
// }
