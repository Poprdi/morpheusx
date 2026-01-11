//! DMA buffer management module.
//!
//! Provides ownership-tracked DMA buffers for safe device communication.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3

pub mod buffer;
pub mod ownership;
pub mod pool;
pub mod region;

// Re-exports
pub use buffer::DmaBuffer;
pub use ownership::BufferOwnership;
pub use pool::{BufferPool, MAX_POOL_SIZE};
pub use region::DmaRegion;
