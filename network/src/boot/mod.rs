//! Boot integration module.
//!
//! Handles the transition from UEFI boot services to bare metal.
//!
//! # Two-Phase Boot Model
//!
//! ## Phase 1: UEFI Boot Services Active
//! - Calibrate TSC using UEFI Stall()
//! - Allocate DMA region via PCI I/O Protocol
//! - Scan PCI for NIC and block devices
//! - Populate BootHandoff structure
//! - Call ExitBootServices()
//!
//! ## Phase 2: Bare Metal (Post-EBS)
//! - Validate BootHandoff
//! - Initialize NIC driver
//! - Initialize block device driver (optional)
//! - Enter main poll loop
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §7

pub mod block_probe;
pub mod handoff;
pub mod init;
pub mod probe;

// Re-exports - Boot handoff
pub use handoff::{
    has_invariant_tsc, read_tsc_raw, BootHandoff, HandoffError, TscCalibration, BLK_TYPE_AHCI,
    BLK_TYPE_NONE, BLK_TYPE_NVME, BLK_TYPE_VIRTIO, HANDOFF_MAGIC, HANDOFF_VERSION,
    NIC_TYPE_BROADCOM, NIC_TYPE_INTEL, NIC_TYPE_NONE, NIC_TYPE_REALTEK, NIC_TYPE_VIRTIO,
};

pub use init::{post_ebs_init, InitError, InitResult, InitializedNicType, TimeoutConfig};

// Re-exports - Network probe
pub use probe::{
    probe_and_create_driver, create_intel_driver, create_virtio_driver, detect_nic_type,
    scan_for_nic, DetectedNic, NicType, ProbeError, ProbeResult,
};

// Re-exports - Block probe
pub use block_probe::{
    probe_and_create_block_driver, probe_unified_block_device, detect_block_device_type,
    scan_for_block_device, find_ahci_controller, AhciInfo, BlockDmaConfig, BlockDeviceType,
    BlockProbeError, BlockProbeResult, DetectedBlockDevice,
};
