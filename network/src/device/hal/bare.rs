//! Bare metal HAL implementation for VirtIO drivers.
//!
//! This HAL uses a pre-allocated static memory pool for DMA operations.
//! It works **after** `ExitBootServices()` or in non-UEFI environments.
//!
//! # Memory Model
//!
//! Uses a simple bump allocator over a static memory region. The memory
//! pool should be allocated before ExitBootServices (via UefiHal) or
//! reserved in the linker script for pure bare metal.
//!
//! # Thread Safety
//!
//! Uses spin locks for thread safety in case of multi-core scenarios.
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::device::hal::BareHal;
//!
//! // Option 1: Initialize with pre-allocated memory (e.g., from UEFI)
//! let pool = unsafe { UefiHal::preallocate_dma_pool(256) }?;
//! unsafe { BareHal::init(pool.0, pool.1, pool.2) };
//!
//! // Option 2: Use a static buffer
//! static mut DMA_POOL: [u8; 1024 * 1024] = [0; 1024 * 1024];
//! unsafe {
//!     let ptr = NonNull::new_unchecked(DMA_POOL.as_mut_ptr());
//!     BareHal::init(DMA_POOL.as_ptr() as usize, ptr, DMA_POOL.len());
//! }
//!
//! // Now VirtIO drivers can be used
//! let net = VirtIONetRaw::<BareHal, _>::new(transport)?;
//! ```

use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use virtio_drivers::{BufferDirection, Hal, PhysAddr, PAGE_SIZE};

use super::common;

/// Maximum number of allocations we track for deallocation.
const MAX_ALLOCATIONS: usize = 256;

/// DMA Pool state.
struct DmaPool {
    /// Base physical address of the pool.
    base_paddr: PhysAddr,
    /// Base virtual address of the pool.
    base_vaddr: *mut u8,
    /// Total size of the pool in bytes.
    total_size: usize,
    /// Current allocation offset (bump pointer).
    offset: AtomicUsize,
    /// Allocation tracking for deallocation.
    allocations: [(PhysAddr, usize); MAX_ALLOCATIONS],
    /// Number of active allocations.
    allocation_count: AtomicUsize,
}

/// Global DMA pool.
static mut DMA_POOL: Option<DmaPool> = None;

/// Initialization flag.
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Simple spinlock for pool access.
static LOCK: AtomicBool = AtomicBool::new(false);

/// Acquire the spinlock.
fn lock() {
    while LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

/// Release the spinlock.
fn unlock() {
    LOCK.store(false, Ordering::Release);
}

/// Bare metal HAL implementation.
///
/// Uses a pre-allocated memory pool for DMA operations.
/// Must be initialized with `init()` before first use.
pub struct BareHal;

impl BareHal {
    /// Initialize the bare metal HAL with a memory pool.
    ///
    /// # Arguments
    ///
    /// * `base_paddr` - Physical base address of the DMA pool
    /// * `base_vaddr` - Virtual base address (NonNull pointer)
    /// * `size` - Size of the pool in bytes (should be page-aligned)
    ///
    /// # Safety
    ///
    /// - The memory region must be valid, physically contiguous, and
    ///   not used by anything else.
    /// - The memory should be identity-mapped or the addresses must
    ///   correctly correspond to each other.
    /// - Must only be called once.
    pub unsafe fn init(base_paddr: PhysAddr, base_vaddr: NonNull<u8>, size: usize) {
        if INITIALIZED.load(Ordering::SeqCst) {
            panic!("BareHal: Already initialized");
        }

        let aligned_size = common::align_down(size, PAGE_SIZE);
        if aligned_size == 0 {
            panic!("BareHal: Pool size must be at least one page");
        }

        // SAFETY: We're in single-threaded init, and we check INITIALIZED
        unsafe {
            DMA_POOL = Some(DmaPool {
                base_paddr,
                base_vaddr: base_vaddr.as_ptr(),
                total_size: aligned_size,
                offset: AtomicUsize::new(0),
                allocations: [(0, 0); MAX_ALLOCATIONS],
                allocation_count: AtomicUsize::new(0),
            });
        }

        INITIALIZED.store(true, Ordering::SeqCst);
    }

    /// Check if the HAL has been initialized.
    pub fn is_initialized() -> bool {
        INITIALIZED.load(Ordering::SeqCst)
    }

    /// Get the remaining free space in the pool.
    pub fn free_space() -> usize {
        if !Self::is_initialized() {
            return 0;
        }

        // SAFETY: We checked initialization
        let pool = unsafe { DMA_POOL.as_ref().unwrap() };
        pool.total_size.saturating_sub(pool.offset.load(Ordering::Relaxed))
    }

    /// Get the total pool size.
    pub fn total_size() -> usize {
        if !Self::is_initialized() {
            return 0;
        }

        // SAFETY: We checked initialization
        let pool = unsafe { DMA_POOL.as_ref().unwrap() };
        pool.total_size
    }

    /// Reset the allocator (dangerous - only use if all allocations are freed).
    ///
    /// # Safety
    ///
    /// All previous allocations must have been freed or abandoned.
    pub unsafe fn reset() {
        if !Self::is_initialized() {
            return;
        }

        lock();
        // SAFETY: We have the lock and checked initialization
        let pool = unsafe { DMA_POOL.as_mut().unwrap() };
        pool.offset.store(0, Ordering::SeqCst);
        pool.allocation_count.store(0, Ordering::SeqCst);
        unlock();
    }

    /// Get the pool for internal operations.
    fn pool() -> &'static DmaPool {
        if !INITIALIZED.load(Ordering::SeqCst) {
            panic!("BareHal: Not initialized. Call BareHal::init() first.");
        }
        // SAFETY: We checked initialization
        unsafe { DMA_POOL.as_ref().unwrap() }
    }

    /// Get mutable pool for internal operations.
    fn pool_mut() -> &'static mut DmaPool {
        if !INITIALIZED.load(Ordering::SeqCst) {
            panic!("BareHal: Not initialized. Call BareHal::init() first.");
        }
        // SAFETY: We checked initialization and caller holds lock
        unsafe { DMA_POOL.as_mut().unwrap() }
    }
}

// SAFETY: We implement the Hal trait requirements correctly:
// - dma_alloc returns valid, aligned, zeroed memory from our pool
// - dma_dealloc tracks deallocations (simple bump allocator can't truly free)
// - Identity mapping is assumed
unsafe impl Hal for BareHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        if pages == 0 {
            panic!("BareHal: Cannot allocate 0 pages");
        }

        lock();

        let pool = Self::pool_mut();
        let size = common::pages_to_bytes(pages);

        // Align current offset to page boundary
        let current = common::align_up(pool.offset.load(Ordering::Relaxed), PAGE_SIZE);
        let new_offset = current + size;

        if new_offset > pool.total_size {
            unlock();
            panic!(
                "BareHal: Out of DMA memory. Requested {} bytes, available {} bytes",
                size,
                pool.total_size - current
            );
        }

        // Calculate addresses
        let paddr = pool.base_paddr + current;
        // SAFETY: Offset is within bounds (checked above)
        let vaddr_ptr = unsafe { pool.base_vaddr.add(current) };
        let vaddr = NonNull::new(vaddr_ptr).expect("BareHal: null vaddr");

        // Update offset
        pool.offset.store(new_offset, Ordering::Release);

        // Track allocation (best effort - wrap around if full)
        let alloc_idx = pool.allocation_count.fetch_add(1, Ordering::Relaxed) % MAX_ALLOCATIONS;
        pool.allocations[alloc_idx] = (paddr, pages);

        unlock();

        // Zero the memory (required by Hal trait)
        // SAFETY: Memory is valid and within our pool
        unsafe {
            core::ptr::write_bytes(vaddr.as_ptr(), 0, size);
        }

        (paddr, vaddr)
    }

    unsafe fn dma_dealloc(_paddr: PhysAddr, _vaddr: NonNull<u8>, _pages: usize) -> i32 {
        // Simple bump allocator doesn't support true deallocation.
        // We just track that it was "freed" for debugging.
        // Memory is reclaimed only on reset().
        //
        // For a real implementation, you'd want a proper allocator
        // (buddy allocator, slab allocator, etc.)
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        // Assume identity mapping for bare metal
        NonNull::new(paddr as *mut u8).expect("BareHal: null MMIO address")
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        // Identity mapping: physical == virtual
        buffer.as_ptr() as *const u8 as PhysAddr
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {
        // No-op for identity mapping
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // Helper to initialize with a test buffer
    fn setup_test_pool(size: usize) {
        // Reset state
        INITIALIZED.store(false, Ordering::SeqCst);
        unsafe { DMA_POOL = None };

        // Create aligned buffer
        let mut buffer = vec![0u8; size + PAGE_SIZE];
        let base = common::align_up(buffer.as_ptr() as usize, PAGE_SIZE);
        let ptr = base as *mut u8;

        // Keep buffer alive (leak it for the test)
        core::mem::forget(buffer);

        unsafe {
            BareHal::init(base, NonNull::new(ptr).unwrap(), size);
        }
    }

    #[test]
    fn test_initialization() {
        setup_test_pool(PAGE_SIZE * 4);
        assert!(BareHal::is_initialized());
        assert_eq!(BareHal::total_size(), PAGE_SIZE * 4);
        assert_eq!(BareHal::free_space(), PAGE_SIZE * 4);
    }

    #[test]
    fn test_allocate_single_page() {
        setup_test_pool(PAGE_SIZE * 4);

        let (paddr, vaddr) = BareHal::dma_alloc(1, BufferDirection::DriverToDevice);

        assert!(paddr > 0);
        assert!(!vaddr.as_ptr().is_null());
        assert_eq!(BareHal::free_space(), PAGE_SIZE * 3);
    }

    #[test]
    fn test_allocate_multiple() {
        setup_test_pool(PAGE_SIZE * 10);

        let (paddr1, _) = BareHal::dma_alloc(2, BufferDirection::DriverToDevice);
        let (paddr2, _) = BareHal::dma_alloc(3, BufferDirection::DeviceToDriver);

        // Allocations should be sequential
        assert_eq!(paddr2, paddr1 + PAGE_SIZE * 2);
        assert_eq!(BareHal::free_space(), PAGE_SIZE * 5);
    }

    #[test]
    fn test_memory_zeroed() {
        setup_test_pool(PAGE_SIZE * 2);

        let (_, vaddr) = BareHal::dma_alloc(1, BufferDirection::DriverToDevice);

        // Check that memory is zeroed
        let slice = unsafe { core::slice::from_raw_parts(vaddr.as_ptr(), PAGE_SIZE) };
        assert!(slice.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_dealloc_returns_zero() {
        setup_test_pool(PAGE_SIZE * 2);

        let (paddr, vaddr) = BareHal::dma_alloc(1, BufferDirection::DriverToDevice);
        let result = unsafe { BareHal::dma_dealloc(paddr, vaddr, 1) };

        assert_eq!(result, 0);
    }

    #[test]
    fn test_reset() {
        setup_test_pool(PAGE_SIZE * 4);

        BareHal::dma_alloc(2, BufferDirection::DriverToDevice);
        assert_eq!(BareHal::free_space(), PAGE_SIZE * 2);

        unsafe { BareHal::reset() };

        assert_eq!(BareHal::free_space(), PAGE_SIZE * 4);
    }

    #[test]
    fn test_share_unshare() {
        setup_test_pool(PAGE_SIZE);

        let mut buffer = [0u8; 64];
        let ptr = NonNull::from(&mut buffer[..]);
        let expected_paddr = buffer.as_ptr() as PhysAddr;

        let paddr = unsafe { BareHal::share(ptr, BufferDirection::DriverToDevice) };
        assert_eq!(paddr, expected_paddr);

        // unshare is a no-op but shouldn't crash
        unsafe { BareHal::unshare(paddr, ptr, BufferDirection::DriverToDevice) };
    }

    #[test]
    fn test_mmio_phys_to_virt() {
        setup_test_pool(PAGE_SIZE);

        let paddr: PhysAddr = 0x1000_0000;
        let vaddr = unsafe { BareHal::mmio_phys_to_virt(paddr, 4096) };

        assert_eq!(vaddr.as_ptr() as PhysAddr, paddr);
    }

    #[test]
    #[should_panic(expected = "Cannot allocate 0 pages")]
    fn test_zero_pages_panics() {
        setup_test_pool(PAGE_SIZE);
        BareHal::dma_alloc(0, BufferDirection::DriverToDevice);
    }
}
