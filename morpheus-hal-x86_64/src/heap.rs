//! `#[global_allocator]` over MemoryRegistry; backed by `linked_list_allocator::Heap`.
//! Grows on demand up to 4x initial size by allocating extending pages adjacent
//! to the current heap end via `AllocateType::Address`.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use spin::Mutex;

use crate::memory::{global_registry_mut, is_registry_initialized, MemoryType, PAGE_SIZE};
use crate::serial::puts;

struct HeapState {
    heap: linked_list_allocator::Heap,
    base: u64,
    size: usize,
    max_size: usize,
}

static HEAP: Mutex<Option<HeapState>> = Mutex::new(None);

/// Fast-path check ahead of the Mutex.
static mut HEAP_INITIALIZED: bool = false;

pub struct HeapAllocator;

impl Default for HeapAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl HeapAllocator {
    pub const fn new() -> Self {
        Self
    }
}

unsafe impl GlobalAlloc for HeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !HEAP_INITIALIZED {
            return ptr::null_mut();
        }

        let mut guard = HEAP.lock();
        let state = match guard.as_mut() {
            Some(s) => s,
            None => return ptr::null_mut(),
        };

        match state.heap.allocate_first_fit(layout) {
            Ok(ptr) => ptr.as_ptr(),
            Err(_) => {
                if try_grow_heap(state, layout.size()) {
                    state
                        .heap
                        .allocate_first_fit(layout)
                        .map(|p| p.as_ptr())
                        .unwrap_or(ptr::null_mut())
                } else {
                    ptr::null_mut()
                }
            },
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
        // Naive: alloc new, copy, dealloc old.
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

/// Reserves contiguous pages at the current heap end so `Heap::extend` stays linear.
unsafe fn try_grow_heap(state: &mut HeapState, needed: usize) -> bool {
    let grow_size = needed.div_ceil(PAGE_SIZE as usize) * PAGE_SIZE as usize;

    if state.size + grow_size > state.max_size {
        puts("[HEAP] cannot grow: would exceed max\n");
        return false;
    }

    if !is_registry_initialized() {
        puts("[HEAP] cannot grow: registry not initialized\n");
        return false;
    }

    let mut registry = global_registry_mut();
    let pages = (grow_size as u64).div_ceil(PAGE_SIZE);
    let extend_addr = state.base + state.size as u64;

    match registry.allocate_pages(
        crate::memory::AllocateType::Address(extend_addr),
        MemoryType::AllocatedHeap,
        pages,
    ) {
        Ok(_) => {
            state.heap.extend(grow_size);
            state.size += grow_size;
            true
        },
        Err(_) => {
            puts("[HEAP] cannot grow: allocation failed\n");
            false
        },
    }
}

/// # Safety
/// Once, after registry init.
pub unsafe fn init_heap(initial_size: usize) -> Result<(), &'static str> {
    if HEAP_INITIALIZED {
        return Err("heap already initialized");
    }

    if !is_registry_initialized() {
        return Err("memory registry not initialized");
    }

    let mut registry = global_registry_mut();

    let size = initial_size.div_ceil(PAGE_SIZE as usize) * PAGE_SIZE as usize;
    let pages = size as u64 / PAGE_SIZE;

    let base = registry
        .allocate_pages(
            crate::memory::AllocateType::AnyPages,
            MemoryType::AllocatedHeap,
            pages,
        )
        .map_err(|_| "failed to allocate heap memory")?;

    let mut heap = linked_list_allocator::Heap::empty();
    heap.init(base as *mut u8, size);

    *HEAP.lock() = Some(HeapState {
        heap,
        base,
        size,
        // Cap growth at 4x initial.
        max_size: size * 4,
    });

    HEAP_INITIALIZED = true;

    Ok(())
}

/// Heap over a caller-owned buffer; cannot grow.
///
/// # Safety
/// `buffer` valid, exclusively owned, ≥ `size` bytes.
pub unsafe fn init_heap_with_buffer(buffer: *mut u8, size: usize) -> Result<(), &'static str> {
    if HEAP_INITIALIZED {
        return Err("heap already initialized");
    }

    if buffer.is_null() || size < crate::memory::PAGE_SIZE as usize {
        return Err("invalid buffer");
    }

    let mut heap = linked_list_allocator::Heap::empty();
    heap.init(buffer, size);

    *HEAP.lock() = Some(HeapState {
        heap,
        base: buffer as u64,
        size,
        max_size: size,
    });

    HEAP_INITIALIZED = true;

    Ok(())
}

pub fn is_heap_initialized() -> bool {
    unsafe { HEAP_INITIALIZED }
}

pub fn heap_stats() -> Option<(usize, usize, usize)> {
    let guard = HEAP.lock();
    guard
        .as_ref()
        .map(|state| (state.size, state.heap.used(), state.heap.free()))
}
