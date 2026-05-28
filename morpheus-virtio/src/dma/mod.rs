//! Ownership-tracked DMA buffers.

pub mod buffer;
pub mod ownership;
pub mod pool;
pub mod region;

pub use buffer::DmaBuffer;
pub use ownership::BufferOwnership;
pub use pool::{BufferPool, MAX_POOL_SIZE};
pub use region::DmaRegion;
