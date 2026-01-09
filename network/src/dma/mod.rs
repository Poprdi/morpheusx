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

// Re-exports
pub use region::DmaRegion;
pub use ownership::BufferOwnership;
pub use buffer::DmaBuffer;
pub use pool::{BufferPool, MAX_POOL_SIZE};
