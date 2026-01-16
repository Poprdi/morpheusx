//! PCI enumeration and configuration access.
//!
//! # Reference
//! - PCI Local Bus Spec 3.0
//! - VirtIO Spec 1.2 §4.1.4

pub mod capability;
pub mod config;

pub use capability::{
    VirtioCapInfo, VirtioPciCaps, VIRTIO_PCI_CAP_COMMON, VIRTIO_PCI_CAP_DEVICE,
    VIRTIO_PCI_CAP_ISR, VIRTIO_PCI_CAP_NOTIFY, VIRTIO_PCI_CAP_PCI_CFG,
};
pub use config::{
    pci_cfg_read8, pci_cfg_read16, pci_cfg_read32,
    pci_cfg_write8, pci_cfg_write16, pci_cfg_write32,
    PciAddr, offset, status,
};
