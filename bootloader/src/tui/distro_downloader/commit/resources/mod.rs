//! Resource allocation and management for bare-metal transition.

pub mod dma;
pub mod handoff;
pub mod stack;

pub use dma::{allocate_dma_region, DMA_SIZE};
pub use handoff::prepare_boot_handoff;
pub use stack::{allocate_stack, STACK_SIZE};
