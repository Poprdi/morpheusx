//! DMA buffer management module.
//!
//! Provides ownership-tracked DMA buffers for safe device communication.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3

pub mod region;
pub mod ownership;
pub mod buffer;
pub mod pool;
