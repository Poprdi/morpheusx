//! VirtIO driver orchestration.

pub mod config;
pub mod init;
pub mod tx;
pub mod rx;
pub mod driver;

// Re-exports
pub use config::{VirtioConfig, features, status, negotiate_features, is_virtio_net};
pub use config::{VIRTIO_VENDOR_ID, VIRTIO_NET_DEVICE_IDS};
pub use init::{virtio_net_init, VirtioInitError};
pub use driver::VirtioNetDriver;
