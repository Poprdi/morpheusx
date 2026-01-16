//! Heap Allocator - Global Allocator backed by MemoryRegistry
//!
//! Provides `#[global_allocator]` support using our MemoryRegistry for
//! physical memory allocation and `linked_list_allocator` for heap management.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                    GlobalAlloc trait                     │
//! │                   (alloc/dealloc/etc)                    │
//! └─────────────────────────────────────────────────────────┘
//!                            │
//!                            ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │                    HeapAllocator                         │
//! │              (linked_list_allocator::Heap)               │
//! └─────────────────────────────────────────────────────────┘
//!                            │
//!                            ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │                   MemoryRegistry                         │
//! │            (allocate_pages for heap growth)              │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // In your main crate:
//! #[global_allocator]
//! static ALLOCATOR: morpheus_hwinit::heap::HeapAllocator =
//!     morpheus_hwinit::heap::HeapAllocator::new();
//!
//! // After memory registry init:
//! unsafe {
//!     morpheus_hwinit::heap::init_heap(4 * 1024 * 1024); // 4MB heap
//! }
//!
//! // Now you can use alloc!
//! let v = alloc::vec![1, 2, 3];
//! ```

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use spin::Mutex;

use crate::memory::{global_registry_mut, is_registry_initialized, MemoryType, PAGE_SIZE};
use crate::serial::{puts, put_hex64, put_hex32};

// ═══════════════════════════════════════════════════════════════════════════
// HEAP STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Heap metadata
struct HeapState {
    /// The actual heap allocator
    heap: linked_list_allocator::Heap,
    /// Base address of heap region
    base: u64,
    /// Current size of heap
    size: usize,
    /// Maximum size we can grow to
    max_size: usize,
}

/// Global heap state
static HEAP: Mutex<Option<HeapState>> = Mutex::new(None);

/// Heap initialized flag (for fast path check)
static mut HEAP_INITIALIZED: bool = false;

// ═══════════════════════════════════════════════════════════════════════════
// HEAP ALLOCATOR
// ═══════════════════════════════════════════════════════════════════════════

/// Global heap allocator.
///
/// This is the type you use with `#[global_allocator]`.
pub struct HeapAllocator;

impl HeapAllocator {
    /// Create new (uninitialized) heap allocator.
    pub const fn new() -> Self {
        Self
    }
}

unsafe impl GlobalAlloc for HeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Fast path: heap not initialized
        if !HEAP_INITIALIZED {
            return ptr::null_mut();
        }

        let mut guard = HEAP.lock();
        let state = match guard.as_mut() {
            Some(s) => s,
            None => return ptr::null_mut(),
        };

        // Try to allocate
        match state.heap.allocate_first_fit(layout) {
            Ok(ptr) => ptr.as_ptr(),
            Err(_) => {
                // Try to grow the heap
                if try_grow_heap(state, layout.size()) {
                    // Retry allocation
                    state.heap
                        .allocate_first_fit(layout)
                        .map(|p| p.as_ptr())
                        .unwrap_or(ptr::null_mut())
                } else {
                    ptr::null_mut()
                }
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() || !HEAP_INITIALIZED {
            return;
        }

        let mut guard = HEAP.lock();
        if let Some(state) = guard.as_mut() {
            if let Some(nn) = NonNull::new(ptr) {
                state.heap.deallocate(nn, layout);
            }
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Simple implementation: alloc new, copy, dealloc old
        let new_layout = match Layout::from_size_align(new_size, layout.align()) {
            Ok(l) => l,
            Err(_) => return ptr::null_mut(),
        };

        let new_ptr = self.alloc(new_layout);
        if !new_ptr.is_null() && !ptr.is_null() {
            let copy_size = layout.size().min(new_size);
            ptr::copy_nonoverlapping(ptr, new_ptr, copy_size);
            self.dealloc(ptr, layout);
        }
        new_ptr
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HEAP GROWTH
// ═══════════════════════════════════════════════════════════════════════════

/// Try to grow the heap by at least `needed` bytes.
///
/// Returns true if growth succeeded.
unsafe fn try_grow_heap(state: &mut HeapState, needed: usize) -> bool {
    // Round up to page size
    let grow_size = ((needed + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize) * PAGE_SIZE as usize;

    // Don't exceed max size
    if state.size + grow_size > state.max_size {
        puts("[HEAP] cannot grow: would exceed max\n");
        return false;
    }

    // Try to allocate more pages from memory registry
    if !is_registry_initialized() {
        puts("[HEAP] cannot grow: registry not initialized\n");
        return false;
    }

    let registry = global_registry_mut();
    let pages = (grow_size as u64 + PAGE_SIZE - 1) / PAGE_SIZE;

    // We need contiguous memory, so allocate at a specific address
    // For simplicity, we extend from the current heap end
    let extend_addr = state.base + state.size as u64;

    match registry.allocate_pages(
        crate::memory::AllocateType::Address(extend_addr),
        MemoryType::AllocatedHeap,
        pages,
    ) {
        Ok(_) => {
            // Extend the heap
            state.heap.extend(grow_size);
            state.size += grow_size;

            puts("[HEAP] grew by ");
            put_hex32(grow_size as u32);
            puts(" bytes, total ");
            put_hex32(state.size as u32);
            puts("\n");

            true
        }
        Err(_) => {
            puts("[HEAP] cannot grow: allocation failed\n");
            false
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize the heap allocator.
///
/// # Arguments
/// - `initial_size`: Initial heap size in bytes (will be rounded up to page size)
///
/// # Safety
/// - Must be called after memory registry is initialized
/// - Must be called exactly once
pub unsafe fn init_heap(initial_size: usize) -> Result<(), &'static str> {
    if HEAP_INITIALIZED {
        return Err("heap already initialized");
    }

    if !is_registry_initialized() {
        return Err("memory registry not initialized");
    }

    let registry = global_registry_mut();

    // Round up to page size
    let size = ((initial_size + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize) * PAGE_SIZE as usize;
    let pages = size as u64 / PAGE_SIZE;

    // Allocate heap memory
    let base = registry.allocate_pages(
        crate::memory::AllocateType::AnyPages,
        MemoryType::AllocatedHeap,
        pages,
    ).map_err(|_| "failed to allocate heap memory")?;

    // Initialize the linked_list_allocator heap
    let mut heap = linked_list_allocator::Heap::empty();
    heap.init(base as *mut u8, size);

    // Store state
    *HEAP.lock() = Some(HeapState {
        heap,
        base,
        size,
        max_size: size * 4, // Allow growing up to 4x initial size
    });

    HEAP_INITIALIZED = true;

    puts("[HEAP] initialized at ");
    put_hex64(base);
    puts(", size ");
    put_hex32(size as u32);
    puts(" bytes\n");

    Ok(())
}

/// Initialize heap with a pre-allocated buffer.
///
/// Use this when you already have a memory region (e.g., from UEFI).
///
/// # Safety
/// - Buffer must be valid and not used for anything else
/// - Buffer must be at least `size` bytes
pub unsafe fn init_heap_with_buffer(buffer: *mut u8, size: usize) -> Result<(), &'static str> {
    if HEAP_INITIALIZED {
        return Err("heap already initialized");
    }

    if buffer.is_null() || size < 4096 {
        return Err("invalid buffer");
    }

    let mut heap = linked_list_allocator::Heap::empty();
    heap.init(buffer, size);

    *HEAP.lock() = Some(HeapState {
        heap,
        base: buffer as u64,
        size,
        max_size: size, // Can't grow a pre-allocated buffer
    });

    HEAP_INITIALIZED = true;

    puts("[HEAP] initialized with buffer at ");
    put_hex64(buffer as u64);
    puts(", size ");
    put_hex32(size as u32);
    puts(" bytes\n");

    Ok(())
}

/// Check if heap is initialized.
pub fn is_heap_initialized() -> bool {
    unsafe { HEAP_INITIALIZED }
}

/// Get heap statistics.
pub fn heap_stats() -> Option<(usize, usize, usize)> {
    let guard = HEAP.lock();
    guard.as_ref().map(|state| {
        (state.size, state.heap.used(), state.heap.free())
    })
}
