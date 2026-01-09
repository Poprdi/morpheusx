//! Driver abstraction module.
//!
//! Provides the NetworkDriver trait and driver implementations.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §8

pub mod traits;
pub mod virtio;
// Future:
// pub mod intel;
// pub mod realtek;
// pub mod broadcom;

// Re-exports
pub use traits::{NetworkDriver, DriverInit, TxError, RxError};
pub use virtio::{VirtioNetDriver, VirtioConfig, VirtioInitError};
