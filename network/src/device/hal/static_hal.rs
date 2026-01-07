//! Static HAL implementation for VirtIO drivers.
//!
//! This HAL is completely firmware-agnostic and uses the `dma-pool` crate
//! for DMA operations. No UEFI, no OS dependencies - pure bare metal.
//!
//! # Device-Agnostic Design
//!
//! This HAL can be used by any device driver that needs DMA memory:
//! - VirtIO (network, block, GPU, etc.)
//! - Intel NICs (future)
//! - Realtek NICs (future)
//! - Any PCIe device requiring DMA
//!
//! # Memory Sources
//!
//! The HAL supports multiple memory initialization strategies:
//! - `init()` - Use static compiled-in pool (default)
//! - `init_discover()` - Find free memory at runtime
//! - `init_external()` - Use caller-provided memory region
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::device::hal::StaticHal;
//!
//! // Initialize once at boot (pick one):
//! StaticHal::init();                    // Static pool
//! StaticHal::init_discover(start, end); // Runtime discovery
//! StaticHal::init_external(addr, size); // External region
//!
//! // Now any driver can use this HAL:
//! let net = VirtIONetRaw::<StaticHal, _>::new(transport)?;
//! let blk = VirtIOBlk::<StaticHal, _>::new(transport)?;
//! ```

use core::ptr::NonNull;
use dma_pool::DmaPool;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

/// Static HAL implementation for device drivers.
///
/// Uses a global DMA pool that can be initialized with static memory,
/// runtime-discovered memory, or externally-provided memory.
///
/// This HAL is designed to be shared across all device drivers in the system.
pub struct StaticHal;

impl StaticHal {
    /// Initialize with the built-in static memory pool.
    ///
    /// This is the simplest option - uses 2MB of compiled-in memory.
    /// Safe to call multiple times (subsequent calls are no-ops).
    #[inline]
    pub fn init() {
        DmaPool::init_static();
    }

    /// Initialize by discovering free memory at runtime.
    ///
    /// Scans the given memory range for large zero-filled regions (code caves,
    /// padding, etc.) that can be used for DMA. Falls back to static pool
    /// if no suitable region is found.
    ///
    /// # Arguments
    ///
    /// * `search_start` - Start of memory range to search
    /// * `search_end` - End of memory range to search
    #[inline]
    pub fn init_discover(search_start: usize, search_end: usize) {
        DmaPool::init_discover(search_start, search_end);
    }

    /// Initialize with an externally-provided memory region.
    ///
    /// Use this when you have a known-good memory region, e.g.:
    /// - Memory reserved via linker script
    /// - Region obtained from firmware before ExitBootServices
    /// - Memory mapped from a specific physical address
    ///
    /// # Safety
    ///
    /// - `base` must be a valid, page-aligned address
    /// - The region must be identity-mapped (phys == virt)
    /// - The region must not be used by anything else
    /// - The region must remain valid for program lifetime
    ///
    /// # Errors
    ///
    /// Returns error if region is too small or misaligned.
    #[inline]
    pub unsafe fn init_external(base: usize, size: usize) -> Result<(), dma_pool::DmaError> {
        DmaPool::init_external(base, size)
    }

    /// Check if the HAL has been initialized.
    #[inline]
    pub fn is_initialized() -> bool {
        DmaPool::is_initialized()
    }

    /// Get remaining free space in bytes.
    #[inline]
    pub fn free_space() -> usize {
        DmaPool::free_space()
    }

    /// Get total pool size in bytes.
    #[inline]
    pub fn total_size() -> usize {
        DmaPool::total_size()
    }

    /// Get pool base address (useful for debugging).
    #[inline]
    pub fn base_address() -> usize {
        DmaPool::base_address()
    }

    /// Reset the allocator (dangerous - only if all allocations freed).
    ///
    /// # Safety
    ///
    /// All previous allocations must be freed or abandoned.
    /// All device drivers must be stopped before calling this.
    #[inline]
    pub unsafe fn reset() {
        DmaPool::reset();
    }
}

// SAFETY: We implement the Hal trait correctly using dma-pool
unsafe impl Hal for StaticHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        DmaPool::alloc_pages(pages)
            .expect("StaticHal: DMA allocation failed")
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        DmaPool::dealloc_pages(paddr, pages);
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        // Identity mapping: physical == virtual
        NonNull::new(paddr as *mut u8).expect("StaticHal: null MMIO address")
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        // Identity mapping: physical == virtual
        // No IOMMU support (pass-through)
        buffer.as_ptr() as *const u8 as PhysAddr
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {
        // No-op for identity mapping without IOMMU
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_idempotent() {
        StaticHal::init();
        assert!(StaticHal::is_initialized());
        StaticHal::init(); // Should be no-op
        assert!(StaticHal::is_initialized());
    }

    #[test]
    fn test_has_memory() {
        StaticHal::init();
        assert!(StaticHal::total_size() > 0);
        assert!(StaticHal::free_space() > 0);
    }
}

