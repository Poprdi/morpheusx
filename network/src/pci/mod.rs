//! PCI enumeration and capability access module.
//!
//! Provides PCI configuration space access and capability chain walking
//! for VirtIO PCI Modern device discovery.
//!
//! # Reference
//! - PCI Local Bus Spec 3.0 §6.7 (Capabilities)
//! - VirtIO Spec 1.2 §4.1.4 (PCI Device Discovery)
//! - ARCHITECTURE_V3.md - PCI layer

pub mod capability;
pub mod config;

pub use capability::{VirtioCapInfo, VirtioPciCaps, VIRTIO_PCI_CAP_COMMON, VIRTIO_PCI_CAP_NOTIFY, 
                     VIRTIO_PCI_CAP_ISR, VIRTIO_PCI_CAP_DEVICE, VIRTIO_PCI_CAP_PCI_CFG};
pub use config::{pci_cfg_read8, pci_cfg_read16, pci_cfg_read32};
