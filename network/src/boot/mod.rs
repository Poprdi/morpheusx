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

pub mod handoff;
pub mod init;

// Re-exports
pub use handoff::{
    BootHandoff, HandoffError, TscCalibration,
    HANDOFF_MAGIC, HANDOFF_VERSION,
    NIC_TYPE_NONE, NIC_TYPE_VIRTIO, NIC_TYPE_INTEL, NIC_TYPE_REALTEK, NIC_TYPE_BROADCOM,
    BLK_TYPE_NONE, BLK_TYPE_VIRTIO, BLK_TYPE_NVME, BLK_TYPE_AHCI,
    has_invariant_tsc, read_tsc_raw,
};

pub use init::{
    InitError, InitResult, TimeoutConfig,
    post_ebs_init,
};
