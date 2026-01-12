//! Post-EBS Global Allocator
//!
//! Uses linked_list_allocator for a battle-tested, no_std heap.
//! Initialized from a static buffer after ExitBootServices.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Static Heap Buffer                       │
//! │                      (1MB default)                          │
//! │                                                             │
//! │  ┌─────────────────────────────────────────────────────┐   │
//! │  │         linked_list_allocator::Heap                 │   │
//! │  │                                                     │   │
//! │  │   Free List: [block] -> [block] -> [block] -> ...  │   │
//! │  │                                                     │   │
//! │  └─────────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // In bare_metal_main, before any allocations:
//! unsafe { crate::alloc::init_heap(); }
//!
//! // Now Vec, Box, String all work:
//! let v = vec![1, 2, 3];
//! let s = String::from("hello");
//! ```
//!
//! # Feature Flags
//!
//! - `post_ebs_allocator`: Enable `#[global_allocator]` attribute.
//!   Only enable this when running standalone post-EBS, not when used
//!   as a library by the bootloader (which has its own UEFI allocator).
//!
//! # Safety
//!
//! - `init_heap()` must be called exactly ONCE before any allocations
//! - Must be called after ExitBootServices (UEFI allocator is gone)
//! - Thread-safety: Uses spin lock internally (safe for single-core post-EBS)

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
use linked_list_allocator::Heap;

/// Heap size: 1MB - sufficient for FAT32 ops, manifest handling, etc.
/// Can be increased if needed.
const HEAP_SIZE: usize = 1024 * 1024;

/// Page-aligned heap buffer wrapper
#[repr(C, align(4096))]
struct AlignedHeapBuffer([u8; HEAP_SIZE]);

/// Static heap buffer - lives in .bss, zero-initialized
static mut HEAP_BUFFER: AlignedHeapBuffer = AlignedHeapBuffer([0u8; HEAP_SIZE]);

/// Locked heap wrapper implementing GlobalAlloc
pub struct LockedHeap {
    inner: spin::Mutex<Heap>,
}

impl LockedHeap {
    /// Create an empty (uninitialized) heap
    pub const fn empty() -> Self {
        Self {
            inner: spin::Mutex::new(Heap::empty()),
        }
    }

    /// Initialize the heap with a memory region
    ///
    /// # Safety
    /// - Must be called exactly once
    /// - Memory region must be valid and not used elsewhere
    pub unsafe fn init(&self, start: *mut u8, size: usize) {
        self.inner.lock().init(start, size);
    }
}

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.inner
            .lock()
            .allocate_first_fit(layout)
            .map(|nn| nn.as_ptr())
            .unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(nn) = NonNull::new(ptr) {
            self.inner.lock().deallocate(nn, layout);
        }
    }
}

// Only use custom allocator when post_ebs_allocator feature is enabled
// This allows the bootloader to use its own UEFI-backed allocator pre-EBS
#[cfg(all(not(test), feature = "post_ebs_allocator"))]
#[global_allocator]
static GLOBAL: LockedHeap = LockedHeap::empty();

// For test builds or when not using as global allocator, still need the static
#[cfg(any(test, not(feature = "post_ebs_allocator")))]
static GLOBAL: LockedHeap = LockedHeap::empty();

/// Track if heap is already initialized
static mut HEAP_INITIALIZED: bool = false;

/// Initialize the heap allocator
///
/// Safe to call multiple times - only initializes once.
/// Should be called as early as possible (start of efi_main).
///
/// # Safety
/// - Must be called BEFORE any allocations (Vec, Box, String, etc.)
/// - Thread-safety: Uses static bool guard, safe for single-core
pub unsafe fn init_heap() {
    if HEAP_INITIALIZED {
        return; // Already initialized
    }

    // Use raw pointer to avoid creating mutable reference to static
    let heap_start = (&raw mut HEAP_BUFFER).cast::<u8>();
    let heap_size = HEAP_SIZE;

    GLOBAL.init(heap_start, heap_size);
    HEAP_INITIALIZED = true;
}

/// Check if heap is initialized
pub fn is_initialized() -> bool {
    unsafe { HEAP_INITIALIZED }
}

/// Get heap statistics for debugging
pub fn heap_stats() -> HeapStats {
    let heap = GLOBAL.inner.lock();
    HeapStats {
        total_size: HEAP_SIZE,
        used: heap.used(),
        free: heap.free(),
    }
}

/// Heap statistics
#[derive(Debug, Clone, Copy)]
pub struct HeapStats {
    pub total_size: usize,
    pub used: usize,
    pub free: usize,
}

impl HeapStats {
    /// Get usage percentage
    pub fn usage_percent(&self) -> u8 {
        if self.total_size == 0 {
            return 0;
        }
        ((self.used * 100) / self.total_size) as u8
    }
}
