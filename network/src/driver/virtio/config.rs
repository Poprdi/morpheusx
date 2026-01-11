//! VirtIO configuration and feature flags.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §4.4, VirtIO Spec §5.1.3

/// VirtIO feature bits.
pub mod features {
    /// VirtIO 1.0+ (modern device).
    pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;

    /// Device has MAC address in config space.
    pub const VIRTIO_NET_F_MAC: u64 = 1 << 5;

    /// Device has link status in config space.
    pub const VIRTIO_NET_F_STATUS: u64 = 1 << 16;

    /// Checksum offload (host handles).
    pub const VIRTIO_NET_F_CSUM: u64 = 1 << 0;

    // ═══════════════════════════════════════════════════════════
    // FORBIDDEN FEATURES - DO NOT NEGOTIATE
    // ═══════════════════════════════════════════════════════════

    /// Guest TSO4 - complicates buffer management.
    pub const VIRTIO_NET_F_GUEST_TSO4: u64 = 1 << 7;

    /// Guest TSO6 - complicates buffer management.
    pub const VIRTIO_NET_F_GUEST_TSO6: u64 = 1 << 8;

    /// Guest UFO - complicates buffer management.
    pub const VIRTIO_NET_F_GUEST_UFO: u64 = 1 << 10;

    /// Mergeable RX buffers - changes header semantics.
    pub const VIRTIO_NET_F_MRG_RXBUF: u64 = 1 << 15;

    /// Control virtqueue - not needed for basic operation.
    pub const VIRTIO_NET_F_CTRL_VQ: u64 = 1 << 17;
}

/// VirtIO device status bits.
pub mod status {
    /// Driver found device.
    pub const ACKNOWLEDGE: u8 = 0x01;
    /// Driver knows how to drive device.
    pub const DRIVER: u8 = 0x02;
    /// Driver ready, device may operate.
    pub const DRIVER_OK: u8 = 0x04;
    /// Feature negotiation complete.
    pub const FEATURES_OK: u8 = 0x08;
    /// Device error, needs reset.
    pub const DEVICE_NEEDS_RESET: u8 = 0x40;
    /// Driver gave up.
    pub const FAILED: u8 = 0x80;
}

/// Required features (device must support, else reject).
pub const REQUIRED_FEATURES: u64 = features::VIRTIO_F_VERSION_1;

/// Desired features (use if available).
pub const DESIRED_FEATURES: u64 = features::VIRTIO_NET_F_MAC | features::VIRTIO_NET_F_STATUS;

/// Forbidden features (never negotiate).
pub const FORBIDDEN_FEATURES: u64 = features::VIRTIO_NET_F_GUEST_TSO4
    | features::VIRTIO_NET_F_GUEST_TSO6
    | features::VIRTIO_NET_F_GUEST_UFO
    | features::VIRTIO_NET_F_MRG_RXBUF
    | features::VIRTIO_NET_F_CTRL_VQ;

/// Feature negotiation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureError {
    /// Device missing required features.
    MissingRequired(u64),
}

/// Negotiate features with device.
///
/// # Arguments
/// - `device_features`: Features advertised by device
///
/// # Returns
/// - `Ok(u64)`: Negotiated feature set
/// - `Err(FeatureError)`: Negotiation failed
pub fn negotiate_features(device_features: u64) -> Result<u64, FeatureError> {
    // Check required features
    if device_features & REQUIRED_FEATURES != REQUIRED_FEATURES {
        return Err(FeatureError::MissingRequired(REQUIRED_FEATURES));
    }

    // Select: required + (desired ∩ device) - forbidden
    let our_features =
        REQUIRED_FEATURES | (DESIRED_FEATURES & device_features) & !FORBIDDEN_FEATURES;

    Ok(our_features)
}

/// VirtIO PCI vendor ID.
pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;

/// VirtIO-net PCI device IDs.
pub const VIRTIO_NET_DEVICE_IDS: &[u16] = &[
    0x1000, // Legacy virtio-net (transitional)
    0x1041, // Modern virtio-net (virtio 1.0+)
];

/// Check if PCI device is VirtIO-net.
pub fn is_virtio_net(vendor: u16, device: u16) -> bool {
    vendor == VIRTIO_VENDOR_ID && VIRTIO_NET_DEVICE_IDS.contains(&device)
}

/// VirtIO driver configuration.
pub struct VirtioConfig {
    /// Pre-allocated DMA region CPU base.
    pub dma_cpu_base: *mut u8,
    /// Pre-allocated DMA region bus address.
    pub dma_bus_base: u64,
    /// DMA region size.
    pub dma_size: usize,
    /// Queue size (number of descriptors).
    pub queue_size: u16,
    /// Buffer size for each queue entry.
    pub buffer_size: usize,
}

impl VirtioConfig {
    /// Default queue size.
    pub const DEFAULT_QUEUE_SIZE: u16 = 32;

    /// Default buffer size (2KB).
    pub const DEFAULT_BUFFER_SIZE: usize = 2048;
}
