//! Boot support module.
//!
//! Provides device probing and driver creation for network and block devices.
//!
//! # Modules
//!
//! - `handoff` - BootHandoff structure (ABI with bootloader)
//! - `probe` - PCI scanning and network driver creation
//! - `block_probe` - PCI scanning and block driver creation
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::boot::probe::{probe_and_create_driver, ProbeResult};
//!
//! // Scan PCI and create appropriate driver
//! let driver = probe_and_create_driver(&dma, tsc_freq)?;
//! ```

pub mod block_probe;
pub mod handoff;
pub mod probe;

// Re-exports - Boot handoff
pub use handoff::{
    has_invariant_tsc, read_tsc_raw, BootHandoff, HandoffError, TscCalibration, BLK_TYPE_AHCI,
    BLK_TYPE_NONE, BLK_TYPE_NVME, BLK_TYPE_VIRTIO, HANDOFF_MAGIC, HANDOFF_VERSION,
    NIC_TYPE_BROADCOM, NIC_TYPE_INTEL, NIC_TYPE_NONE, NIC_TYPE_REALTEK, NIC_TYPE_VIRTIO,
    TRANSPORT_MMIO, TRANSPORT_PCI_MODERN,
};

// Re-exports - Network probe
pub use probe::{
    create_intel_driver, create_virtio_driver, detect_nic_type, probe_and_create_driver,
    scan_for_nic, DetectedNic, NicType, ProbeError, ProbeResult,
};

// Re-exports - Block probe
pub use block_probe::{
    detect_block_device_type, find_ahci_controller, probe_and_create_block_driver,
    probe_unified_block_device, scan_for_block_device, AhciInfo, BlockDeviceType, BlockDmaConfig,
    BlockProbeError, BlockProbeResult, DetectedBlockDevice,
};
