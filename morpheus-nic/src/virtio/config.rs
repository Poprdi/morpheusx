//! VirtIO 1.1 feature/status flags and negotiation. §5.1 (net).

pub mod features {
    pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;

    pub const VIRTIO_NET_F_MAC: u64 = 1 << 5;
    pub const VIRTIO_NET_F_STATUS: u64 = 1 << 16;
    /// Host-side checksum offload.
    pub const VIRTIO_NET_F_CSUM: u64 = 1 << 0;
    pub const VIRTIO_NET_F_GUEST_TSO4: u64 = 1 << 7;
    pub const VIRTIO_NET_F_GUEST_TSO6: u64 = 1 << 8;
    pub const VIRTIO_NET_F_GUEST_UFO: u64 = 1 << 10;
    /// Changes RX header semantics; rejected.
    pub const VIRTIO_NET_F_MRG_RXBUF: u64 = 1 << 15;
    pub const VIRTIO_NET_F_CTRL_VQ: u64 = 1 << 17;
}

pub mod status {
    pub const ACKNOWLEDGE: u8 = 0x01;
    pub const DRIVER: u8 = 0x02;
    pub const DRIVER_OK: u8 = 0x04;
    pub const FEATURES_OK: u8 = 0x08;
    pub const DEVICE_NEEDS_RESET: u8 = 0x40;
    pub const FAILED: u8 = 0x80;
}

pub const REQUIRED_FEATURES: u64 = features::VIRTIO_F_VERSION_1;

pub const DESIRED_FEATURES: u64 = features::VIRTIO_NET_F_MAC | features::VIRTIO_NET_F_STATUS;

pub const FORBIDDEN_FEATURES: u64 = features::VIRTIO_NET_F_GUEST_TSO4
    | features::VIRTIO_NET_F_GUEST_TSO6
    | features::VIRTIO_NET_F_GUEST_UFO
    | features::VIRTIO_NET_F_MRG_RXBUF
    | features::VIRTIO_NET_F_CTRL_VQ;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureError {
    MissingRequired(u64),
}

/// Returns `required | (desired ∩ device)` minus `forbidden`.
pub fn negotiate_features(device_features: u64) -> Result<u64, FeatureError> {
    if device_features & REQUIRED_FEATURES != REQUIRED_FEATURES {
        return Err(FeatureError::MissingRequired(REQUIRED_FEATURES));
    }

    let our_features =
        REQUIRED_FEATURES | (DESIRED_FEATURES & device_features) & !FORBIDDEN_FEATURES;

    Ok(our_features)
}

pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;

pub const VIRTIO_NET_DEVICE_IDS: &[u16] = &[
    0x1000, // Legacy/transitional.
    0x1041, // Modern (virtio 1.0+).
];

pub fn is_virtio_net(vendor: u16, device: u16) -> bool {
    vendor == VIRTIO_VENDOR_ID && VIRTIO_NET_DEVICE_IDS.contains(&device)
}

pub struct VirtioConfig {
    pub dma_cpu_base: *mut u8,
    pub dma_bus_base: u64,
    pub dma_size: usize,
    pub queue_size: u16,
    pub buffer_size: usize,
}

impl VirtioConfig {
    pub const DEFAULT_QUEUE_SIZE: u16 = 32;
    pub const DEFAULT_BUFFER_SIZE: usize = 2048;
}
