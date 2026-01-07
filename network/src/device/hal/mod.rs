//! Hardware Abstraction Layer for VirtIO drivers.
//!
//! This module provides the `Hal` trait implementation required by the
//! `virtio-drivers` crate. We use a single, firmware-agnostic implementation
//! backed by the `dma-pool` crate.
//!
//! # Design Philosophy
//!
//! The HAL is completely firmware-agnostic - no UEFI, no OS dependencies.
//! It uses a static memory pool compiled into the binary for DMA operations.
//! This eliminates all firmware quirks and compatibility issues.
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::device::hal::StaticHal;
//!
//! // Initialize the HAL (call once at startup)
//! StaticHal::init();
//!
//! // Now VirtIO drivers can be used
//! let net = VirtIONetRaw::<StaticHal, _>::new(transport)?;
//! ```

pub mod static_hal;

pub use static_hal::StaticHal;

// Re-export dma-pool utilities for convenience
pub use dma_pool::{
    DmaPool, DmaError, MemoryRegion, MemoryDiscovery,
    PAGE_SIZE, align_up, align_down, pages_to_bytes, bytes_to_pages,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_functions() {
        assert_eq!(align_up(0, 4096), 0);
        assert_eq!(align_up(1, 4096), 4096);
        assert_eq!(align_up(4096, 4096), 4096);
        assert_eq!(align_down(4097, 4096), 4096);
    }

    #[test]
    fn test_page_conversions() {
        assert_eq!(pages_to_bytes(1), PAGE_SIZE);
        assert_eq!(bytes_to_pages(PAGE_SIZE), 1);
        assert_eq!(bytes_to_pages(PAGE_SIZE + 1), 2);
    }
}