//! Shared data types module.
//!
//! Contains all #[repr(C)] structures that are shared between Rust and ASM,
//! as well as other common type definitions.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.6, ARCHITECTURE_V3.md

pub mod repr_c;
pub mod virtio_hdr;
pub mod ethernet;
pub mod result;
