//! VirtIO configuration and feature flags.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §4.4

// TODO: Implement VirtIO configuration
//
// pub mod features {
//     pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;
//     pub const VIRTIO_NET_F_MAC: u64 = 1 << 5;
//     pub const VIRTIO_NET_F_STATUS: u64 = 1 << 16;
//     
//     // Forbidden features
//     pub const VIRTIO_NET_F_GUEST_TSO4: u64 = 1 << 7;
//     pub const VIRTIO_NET_F_GUEST_TSO6: u64 = 1 << 8;
//     pub const VIRTIO_NET_F_MRG_RXBUF: u64 = 1 << 15;
// }
//
// pub const REQUIRED_FEATURES: u64 = features::VIRTIO_F_VERSION_1;
// pub const DESIRED_FEATURES: u64 = features::VIRTIO_NET_F_MAC | features::VIRTIO_NET_F_STATUS;
// pub const FORBIDDEN_FEATURES: u64 = features::VIRTIO_NET_F_GUEST_TSO4 | ...;
//
// pub fn negotiate_features(device_features: u64) -> Result<u64, FeatureError> { ... }
