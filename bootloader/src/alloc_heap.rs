//! Standalone `linked_list_allocator` heap over a 1 MB static buffer.
//!
//! NOT the global allocator — that is `uefi_allocator::ALLOCATOR`
//! (HybridAllocator). This is a self-contained pool for callers wanting
//! their own backing storage.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
use linked_list_allocator::Heap;

const HEAP_SIZE: usize = 1024 * 1024;

#[repr(C, align(4096))]
struct AlignedHeapBuffer([u8; HEAP_SIZE]);

static mut HEAP_BUFFER: AlignedHeapBuffer = AlignedHeapBuffer([0u8; HEAP_SIZE]);

pub struct LockedHeap {
    inner: spin::Mutex<Heap>,
}

impl LockedHeap {
    pub const fn empty() -> Self {
        Self {
            inner: spin::Mutex::new(Heap::empty()),
        }
    }

    /// # Safety
    /// Call once with a region that isn't otherwise aliased.
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

static GLOBAL: LockedHeap = LockedHeap::empty();

static mut HEAP_INITIALIZED: bool = false;

/// Idempotent. Call before the first allocation.
///
/// # Safety
/// Single-core only (no real sync on the guard).
pub unsafe fn init_heap() {
    if HEAP_INITIALIZED {
        return;
    }

    let heap_start = (&raw mut HEAP_BUFFER).cast::<u8>();
    GLOBAL.init(heap_start, HEAP_SIZE);
    HEAP_INITIALIZED = true;
}

pub fn is_initialized() -> bool {
    unsafe { HEAP_INITIALIZED }
}

pub fn heap_stats() -> HeapStats {
    let heap = GLOBAL.inner.lock();
    HeapStats {
        total_size: HEAP_SIZE,
        used: heap.used(),
        free: heap.free(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HeapStats {
    pub total_size: usize,
    pub used: usize,
    pub free: usize,
}

impl HeapStats {
    pub fn usage_percent(&self) -> u8 {
        if self.total_size == 0 {
            return 0;
        }
        ((self.used * 100) / self.total_size) as u8
    }
}
