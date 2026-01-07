//! Hardware Abstraction Layer for VirtIO drivers.
//!
//! This module provides the `Hal` trait implementation required by the
//! `virtio-drivers` crate. We use a single, firmware-agnostic implementation.
//!
//! # Design Philosophy
//!
//! The HAL is completely firmware-agnostic - no UEFI, no OS dependencies.
//! It uses a static memory pool compiled into the binary for DMA operations.
//! This eliminates all firmware quirks and compatibility issues.
//!
//! # Architecture
//!
//! The HAL provides:
//! - DMA memory allocation (physically contiguous from static pool)
//! - Physical-to-virtual address translation (identity mapping)
//! - Memory sharing for IOMMU (pass-through, no IOMMU support needed)
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

extern crate alloc;

pub mod static_hal;

pub use static_hal::StaticHal;

use core::ptr::NonNull;
use virtio_drivers::{BufferDirection, Hal, PhysAddr, PAGE_SIZE};

/// Common HAL utilities.
pub mod common {
    use super::*;

    /// Align a value up to the given alignment.
    #[inline]
    pub const fn align_up(val: usize, align: usize) -> usize {
        (val + align - 1) & !(align - 1)
    }

    /// Align a value down to the given alignment.
    #[inline]
    pub const fn align_down(val: usize, align: usize) -> usize {
        val & !(align - 1)
    }

    /// Convert pages to bytes.
    #[inline]
    pub const fn pages_to_bytes(pages: usize) -> usize {
        pages * PAGE_SIZE
    }

    /// Convert bytes to pages (rounded up).
    #[inline]
    pub const fn bytes_to_pages(bytes: usize) -> usize {
        align_up(bytes, PAGE_SIZE) / PAGE_SIZE
    }
}

/// A mock HAL for testing purposes.
///
/// This HAL uses the global allocator and assumes identity mapping.
/// Only use for unit tests, not real hardware!
#[cfg(test)]
pub struct MockHal;

#[cfg(test)]
unsafe impl Hal for MockHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        use alloc::alloc::{alloc_zeroed, Layout};

        let size = common::pages_to_bytes(pages);
        let layout = Layout::from_size_align(size, PAGE_SIZE).unwrap();

        // SAFETY: Layout is valid and non-zero sized
        let ptr = unsafe { alloc_zeroed(layout) };

        if ptr.is_null() {
            panic!("MockHal: DMA allocation failed");
        }

        let vaddr = unsafe { NonNull::new_unchecked(ptr) };
        let paddr = ptr as PhysAddr;

        (paddr, vaddr)
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        use alloc::alloc::{dealloc, Layout};

        let size = common::pages_to_bytes(pages);
        let layout = Layout::from_size_align(size, PAGE_SIZE).unwrap();
        let ptr = paddr as *mut u8;

        // SAFETY: Caller guarantees this was allocated by dma_alloc
        unsafe { dealloc(ptr, layout) };

        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        // Identity mapping for tests
        NonNull::new(paddr as *mut u8).expect("null MMIO address")
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        // Identity mapping - physical == virtual
        buffer.as_ptr() as *const u8 as PhysAddr
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {
        // No-op for identity mapping
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_up() {
        assert_eq!(common::align_up(0, 4096), 0);
        assert_eq!(common::align_up(1, 4096), 4096);
        assert_eq!(common::align_up(4095, 4096), 4096);
        assert_eq!(common::align_up(4096, 4096), 4096);
        assert_eq!(common::align_up(4097, 4096), 8192);
    }

    #[test]
    fn test_align_down() {
        assert_eq!(common::align_down(0, 4096), 0);
        assert_eq!(common::align_down(1, 4096), 0);
        assert_eq!(common::align_down(4095, 4096), 0);
        assert_eq!(common::align_down(4096, 4096), 4096);
        assert_eq!(common::align_down(4097, 4096), 4096);
    }

    #[test]
    fn test_pages_to_bytes() {
        assert_eq!(common::pages_to_bytes(0), 0);
        assert_eq!(common::pages_to_bytes(1), PAGE_SIZE);
        assert_eq!(common::pages_to_bytes(10), 10 * PAGE_SIZE);
    }

    #[test]
    fn test_bytes_to_pages() {
        assert_eq!(common::bytes_to_pages(0), 0);
        assert_eq!(common::bytes_to_pages(1), 1);
        assert_eq!(common::bytes_to_pages(PAGE_SIZE), 1);
        assert_eq!(common::bytes_to_pages(PAGE_SIZE + 1), 2);
    }

    #[test]
    fn test_mock_hal_alloc_dealloc() {
        let (paddr, vaddr) = MockHal::dma_alloc(1, BufferDirection::DriverToDevice);
        assert!(!vaddr.as_ptr().is_null());
        assert_eq!(paddr, vaddr.as_ptr() as PhysAddr);

        // SAFETY: We just allocated this
        let result = unsafe { MockHal::dma_dealloc(paddr, vaddr, 1) };
        assert_eq!(result, 0);
    }

    #[test]
    fn test_mock_hal_share_unshare() {
        let mut buffer = [0u8; 64];
        let ptr = NonNull::from(&mut buffer[..]);
        let expected_paddr = buffer.as_ptr() as PhysAddr;

        // SAFETY: Valid buffer
        let paddr = unsafe { MockHal::share(ptr, BufferDirection::DriverToDevice) };
        assert_eq!(paddr, expected_paddr);

        // SAFETY: Valid buffer and paddr from share
        unsafe { MockHal::unshare(paddr, ptr, BufferDirection::DriverToDevice) };
    }
}
