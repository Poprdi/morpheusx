//! Buffer ownership state machine.
//!
//! # State Machine
//! ```text
//!     FREE ──alloc()──> DRIVER_OWNED ──submit()──> DEVICE_OWNED
//!       ▲                     │                         │
//!       └────free()───────────┴─────poll_complete()─────┘
//! ```
//!
//! INVARIANT: Accessing DEVICE_OWNED buffer is instant UB.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3.4

// TODO: Implement BufferOwnership
//
// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
// pub enum BufferOwnership {
//     Free,
//     DriverOwned,
//     DeviceOwned,
// }
