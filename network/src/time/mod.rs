//! Time and timing module.
//!
//! TSC-based timing with calibrated timeouts.
//!
//! Note: Actual timing implementation is in:
//! - `state/mod.rs` - TscTimestamp for state machines
//! - `boot/init.rs` - TimeoutConfig for boot timeouts
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5.7

// Timing functionality is implemented inline in state machines.
// Future: Consider consolidating timing utilities here.
