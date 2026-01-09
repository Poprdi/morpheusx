//! TCP connection state machine.
//!
//! Non-blocking TCP connection establishment.
//!
//! # States
//! Closed → Connecting → Established | Error
//! Established → Closing → Closed | Error
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5.4

// TODO: Implement TcpConnState
//
// pub enum TcpConnState {
//     Closed,
//     Connecting { socket: SocketHandle, remote: (Ipv4Addr, u16), start_tsc: u64 },
//     Established { socket: SocketHandle },
//     Closing { socket: SocketHandle, start_tsc: u64 },
//     Error { error: TcpError },
// }
//
// impl TcpConnState {
//     pub fn new() -> Self { ... }
//     pub fn connect(&mut self, ...) -> Result<(), TcpError> { ... }
//     pub fn step(&mut self, ...) -> StepResult { ... }
//     pub fn close(&mut self, ...) { ... }
//     pub fn socket(&self) -> Option<SocketHandle> { ... }
// }
