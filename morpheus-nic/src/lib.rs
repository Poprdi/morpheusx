//! NIC drivers (Intel e1000e, virtio-net), device probe, and `BootHandoff` ABI.

#![no_std]
extern crate alloc;

pub mod asm;
pub mod boot_handoff;
pub mod boot_probe;
pub mod device;
pub mod intel;
pub(crate) mod serial;
pub mod traits;
pub mod unified;
pub mod virtio;
