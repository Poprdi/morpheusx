//! DHCP client state machine.
//!
//! Wraps smoltcp's DHCP client with non-blocking state tracking.
//!
//! # States
//! Init → Discovering → Bound | Failed
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5.3

// TODO: Implement DhcpState
//
// pub enum DhcpState {
//     Init,
//     Discovering { start_tsc: u64 },
//     Bound { ip: Ipv4Addr, gateway: Option<Ipv4Addr>, dns: Option<Ipv4Addr> },
//     Failed { error: DhcpError },
// }
//
// impl DhcpState {
//     pub fn new() -> Self { ... }
//     pub fn start(&mut self, now_tsc: u64) { ... }
//     pub fn step(&mut self, iface: &mut Interface, now_tsc: u64, timeouts: &TimeoutConfig) -> StepResult { ... }
//     pub fn is_bound(&self) -> bool { ... }
//     pub fn ip(&self) -> Option<Ipv4Addr> { ... }
// }
