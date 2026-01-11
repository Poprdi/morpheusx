//! Driver abstraction module.
//!
//! Provides the NetworkDriver trait and driver implementations.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §8

pub mod block_io_adapter;
pub mod block_traits;
pub mod traits;
pub mod virtio;
pub mod virtio_blk;
// Future:
// pub mod intel;
// pub mod realtek;
// pub mod broadcom;

// Re-exports - Network
pub use traits::{DriverInit, NetworkDriver, RxError, TxError};
pub use virtio::{VirtioConfig, VirtioInitError, VirtioNetDriver};

// Re-exports - Block
pub use block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};
pub use virtio_blk::{VirtioBlkConfig, VirtioBlkDriver, VirtioBlkInitError};

// Re-exports - BlockIo adapter (for filesystem compatibility)
pub use block_io_adapter::{BlockIoError, VirtioBlkBlockIo};
