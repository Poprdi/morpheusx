//! Static HAL implementation for VirtIO drivers.
//!
//! This HAL is completely firmware-agnostic and uses a static memory pool
//! for DMA operations. No UEFI, no OS dependencies - pure bare metal.
//!
//! # Memory Model
//!
//! Uses a simple bump allocator over a static memory region. The memory
//! is compiled into the binary and uses identity mapping (phys == virt).
//!
//! # Thread Safety
//!
//! Uses spin locks for thread safety in multi-core scenarios.
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::device::hal::StaticHal;
//!
//! // Initialize the HAL (uses built-in static pool)
//! StaticHal::init();
//!
//! // Now VirtIO drivers can be used
//! let net = VirtIONetRaw::<StaticHal, _>::new(transport)?;
//! ```

use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use virtio_drivers::{BufferDirection, Hal, PhysAddr, PAGE_SIZE};

use super::common;

/// Size of the static DMA pool (2MB should be plenty for network buffers).
/// VirtIO net needs ~64KB for queues + packet buffers.
const DMA_POOL_SIZE: usize = 2 * 1024 * 1024;

/// Number of pages in the pool.
const DMA_POOL_PAGES: usize = DMA_POOL_SIZE / PAGE_SIZE;

/// Maximum number of allocations we track.
const MAX_ALLOCATIONS: usize = 64;

/// Static DMA memory pool - page aligned.
/// This memory is compiled into the binary.
#[repr(C, align(4096))]
struct DmaPoolStorage {
    data: [u8; DMA_POOL_SIZE],
}

/// The actual static pool storage.
static mut DMA_POOL_STORAGE: DmaPoolStorage = DmaPoolStorage {
    data: [0u8; DMA_POOL_SIZE],
};

/// Allocation tracking entry.
#[derive(Clone, Copy)]
struct Allocation {
    offset: usize,  // Offset from pool base
    pages: usize,   // Number of pages
    in_use: bool,
}

impl Allocation {
    const fn empty() -> Self {
        Self {
            offset: 0,
            pages: 0,
            in_use: false,
        }
    }
}

/// Pool state.
struct PoolState {
    /// Current bump pointer offset.
    bump_offset: AtomicUsize,
    /// Allocation tracking.
    allocations: [Allocation; MAX_ALLOCATIONS],
    /// Number of allocations.
    alloc_count: AtomicUsize,
}

/// Global pool state.
static mut POOL_STATE: PoolState = PoolState {
    bump_offset: AtomicUsize::new(0),
    allocations: [Allocation::empty(); MAX_ALLOCATIONS],
    alloc_count: AtomicUsize::new(0),
};

/// Initialization flag.
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Simple spinlock for pool access.
static LOCK: AtomicBool = AtomicBool::new(false);

/// Acquire the spinlock.
#[inline]
fn lock() {
    while LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

/// Release the spinlock.
#[inline]
fn unlock() {
    LOCK.store(false, Ordering::Release);
}

/// Static HAL implementation.
///
/// Uses a compiled-in static memory pool for DMA operations.
/// Completely firmware-agnostic - works anywhere with identity mapping.
pub struct StaticHal;

impl StaticHal {
    /// Initialize the static HAL.
    ///
    /// This just marks the HAL as ready to use. The memory pool is
    /// statically allocated and doesn't need runtime setup.
    ///
    /// Safe to call multiple times (subsequent calls are no-ops).
    pub fn init() {
        if INITIALIZED.swap(true, Ordering::SeqCst) {
            return; // Already initialized
        }
        
        // Zero the pool on first init (defensive)
        // SAFETY: Single-threaded init, pool is valid static memory
        unsafe {
            core::ptr::write_bytes(DMA_POOL_STORAGE.data.as_mut_ptr(), 0, DMA_POOL_SIZE);
        }
    }

    /// Check if the HAL has been initialized.
    #[inline]
    pub fn is_initialized() -> bool {
        INITIALIZED.load(Ordering::SeqCst)
    }

    /// Get the base physical address of the pool.
    /// In bare metal with identity mapping, this equals the virtual address.
    #[inline]
    fn pool_base() -> usize {
        // SAFETY: Static storage has stable address
        unsafe { DMA_POOL_STORAGE.data.as_ptr() as usize }
    }

    /// Get remaining free space in bytes.
    pub fn free_space() -> usize {
        if !Self::is_initialized() {
            return 0;
        }
        // SAFETY: Reading atomic is safe
        let offset = unsafe { POOL_STATE.bump_offset.load(Ordering::Relaxed) };
        DMA_POOL_SIZE.saturating_sub(offset)
    }

    /// Get total pool size in bytes.
    pub const fn total_size() -> usize {
        DMA_POOL_SIZE
    }

    /// Reset the allocator (dangerous - only if all allocations freed).
    ///
    /// # Safety
    ///
    /// All previous allocations must be freed or abandoned.
    pub unsafe fn reset() {
        lock();
        POOL_STATE.bump_offset.store(0, Ordering::SeqCst);
        POOL_STATE.alloc_count.store(0, Ordering::SeqCst);
        for alloc in POOL_STATE.allocations.iter_mut() {
            *alloc = Allocation::empty();
        }
        unlock();
    }
}

// SAFETY: We implement the Hal trait correctly:
// - dma_alloc returns valid, aligned, zeroed memory from static pool
// - dma_dealloc tracks deallocations
// - Identity mapping is correct for bare metal
unsafe impl Hal for StaticHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        if !INITIALIZED.load(Ordering::SeqCst) {
            panic!("StaticHal: Not initialized. Call StaticHal::init() first.");
        }

        if pages == 0 {
            panic!("StaticHal: Cannot allocate 0 pages");
        }

        let size = common::pages_to_bytes(pages);
        
        lock();
        
        // SAFETY: We hold the lock
        let offset = unsafe { POOL_STATE.bump_offset.load(Ordering::Relaxed) };
        let aligned_offset = common::align_up(offset, PAGE_SIZE);
        let new_offset = aligned_offset + size;
        
        if new_offset > DMA_POOL_SIZE {
            unlock();
            panic!(
                "StaticHal: Out of DMA memory. Requested {} pages ({} bytes), \
                 available {} bytes",
                pages, size, DMA_POOL_SIZE - offset
            );
        }
        
        // SAFETY: We hold the lock and bounds are checked
        unsafe {
            POOL_STATE.bump_offset.store(new_offset, Ordering::SeqCst);
            
            // Track allocation
            let alloc_idx = POOL_STATE.alloc_count.fetch_add(1, Ordering::SeqCst);
            if alloc_idx < MAX_ALLOCATIONS {
                POOL_STATE.allocations[alloc_idx] = Allocation {
                    offset: aligned_offset,
                    pages,
                    in_use: true,
                };
            }
        }
        
        unlock();
        
        // Calculate addresses
        let paddr = Self::pool_base() + aligned_offset;
        let vaddr_ptr = paddr as *mut u8;
        
        // Zero the memory (required by Hal trait)
        // SAFETY: Memory is within our static pool and we just allocated it
        unsafe {
            core::ptr::write_bytes(vaddr_ptr, 0, size);
        }
        
        let vaddr = NonNull::new(vaddr_ptr).expect("StaticHal: null pointer from pool");
        
        (paddr as PhysAddr, vaddr)
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        // With bump allocation, we can't truly free memory.
        // Just mark it as not in use for tracking purposes.
        
        let pool_base = Self::pool_base();
        let offset = (paddr as usize).saturating_sub(pool_base);
        
        lock();
        
        // SAFETY: We hold the lock
        unsafe {
            for alloc in POOL_STATE.allocations.iter_mut() {
                if alloc.in_use && alloc.offset == offset && alloc.pages == pages {
                    alloc.in_use = false;
                    break;
                }
            }
        }
        
        unlock();
        
        0 // Success
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        // Identity mapping: physical == virtual
        NonNull::new(paddr as *mut u8).expect("StaticHal: null MMIO address")
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        // Identity mapping: physical == virtual
        // No IOMMU in simple bare metal case
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
    fn test_pool_constants() {
        assert_eq!(DMA_POOL_SIZE, 2 * 1024 * 1024);
        assert_eq!(DMA_POOL_PAGES, 512);
        assert_eq!(PAGE_SIZE, 4096);
    }

    #[test]
    fn test_init_idempotent() {
        // Reset state for test
        INITIALIZED.store(false, Ordering::SeqCst);
        
        assert!(!StaticHal::is_initialized());
        StaticHal::init();
        assert!(StaticHal::is_initialized());
        
        // Second init should be no-op
        StaticHal::init();
        assert!(StaticHal::is_initialized());
    }

    #[test]
    fn test_free_space() {
        INITIALIZED.store(false, Ordering::SeqCst);
        assert_eq!(StaticHal::free_space(), 0); // Not initialized
        
        StaticHal::init();
        assert_eq!(StaticHal::free_space(), DMA_POOL_SIZE);
    }

    #[test]
    fn test_total_size() {
        assert_eq!(StaticHal::total_size(), DMA_POOL_SIZE);
    }
}
