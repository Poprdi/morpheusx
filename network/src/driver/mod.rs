//! Driver abstraction module.
//!
//! Provides the NetworkDriver trait and driver implementations.
//!
//! # Architecture
//!
//! MorpheusX drivers follow an ASM-first pattern:
//! - All hardware access via hand-written x86_64 assembly
//! - Rust code handles orchestration, state, and error handling
//! - Unified abstractions for QEMU ↔ real hardware parity
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §8

pub mod ahci;
pub mod block_io_adapter;
pub mod block_traits;
pub mod intel;
pub mod traits;
pub mod unified;
pub mod unified_block_io;
pub mod virtio;
pub mod virtio_blk;
// Future:
// pub mod realtek;
// pub mod broadcom;

// Re-exports - Network
pub use traits::{DriverInit, NetworkDriver, RxError, TxError};
pub use virtio::{VirtioConfig, VirtioInitError, VirtioNetDriver};

// Re-exports - Intel e1000e
pub use intel::{E1000eConfig, E1000eDriver, E1000eError, IntelNicInfo};

// Re-exports - Unified Network Driver
pub use unified::{UnifiedDriverError, UnifiedNetworkDriver};

// Re-exports - Block (VirtIO)
pub use block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};
pub use virtio_blk::{VirtioBlkConfig, VirtioBlkDriver, VirtioBlkInitError};

// Re-exports - Block (AHCI/SATA for real hardware)
pub use ahci::{AhciConfig, AhciDriver, AhciInitError};

// Re-exports - BlockIo adapters (for filesystem compatibility)
pub use block_io_adapter::{BlockIoError, VirtioBlkBlockIo};
pub use unified_block_io::{GenericBlockIo, UnifiedBlockIo, UnifiedBlockIoError};

