//! VirtIO driver orchestration.

pub mod config;
pub mod driver;
pub mod init;
pub mod rx;
pub mod transport;
pub mod tx;

// Re-exports
pub use config::{features, is_virtio_net, negotiate_features, status, VirtioConfig};
pub use config::{VIRTIO_NET_DEVICE_IDS, VIRTIO_VENDOR_ID};
pub use driver::VirtioNetDriver;
pub use init::{virtio_net_init, virtio_net_init_transport, VirtioInitError};
pub use transport::{PciModernConfig, TransportType, VirtioTransport};
