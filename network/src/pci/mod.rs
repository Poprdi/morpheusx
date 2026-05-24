//! PCI enumeration and capability access module.
//!
//! Provides PCI configuration space access and capability chain walking
//! for VirtIO PCI Modern device discovery.

pub mod capability;
pub mod config;

pub use capability::{
    VirtioCapInfo, VirtioPciCaps, VIRTIO_PCI_CAP_COMMON, VIRTIO_PCI_CAP_DEVICE, VIRTIO_PCI_CAP_ISR,
    VIRTIO_PCI_CAP_NOTIFY, VIRTIO_PCI_CAP_PCI_CFG,
};
pub use config::{pci_cfg_read16, pci_cfg_read32, pci_cfg_read8};
