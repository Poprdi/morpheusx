//! Shared VirtIO transport + virtqueue infrastructure.
//!
//! Consumed by `morpheus-block` (virtio_blk) and `morpheus-nic` (virtio-net).

#![no_std]

pub mod asm;
pub mod dma;
pub mod transport;
pub mod types;
