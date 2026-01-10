//! Driver abstraction module.
//!
//! Provides the NetworkDriver trait and driver implementations.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §8

pub mod traits;
pub mod block_traits;
pub mod block_io_adapter;
pub mod virtio;
pub mod virtio_blk;
// Future:
// pub mod intel;
// pub mod realtek;
// pub mod broadcom;

// Re-exports - Network
pub use traits::{NetworkDriver, DriverInit, TxError, RxError};
pub use virtio::{VirtioNetDriver, VirtioConfig, VirtioInitError};

// Re-exports - Block
pub use block_traits::{BlockDriver, BlockDriverInit, BlockError, BlockCompletion, BlockDeviceInfo};
pub use virtio_blk::{VirtioBlkDriver, VirtioBlkConfig, VirtioBlkInitError};

// Re-exports - BlockIo adapter (for filesystem compatibility)
pub use block_io_adapter::{VirtioBlkBlockIo, BlockIoError};

