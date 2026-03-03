//! Userspace buddy heap. Intrusive free-lists, XOR buddy math, 8 MiB arenas
//! from SYS_MMAP. Registered as `#[global_allocator]` — Vec/Box/String just work.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, Ordering};

pub const MIN_ALLOC: usize = 16; // smallest block & alignment
const MIN_ALLOC_SHIFT: usize = 4; // log2(16)
const MAX_ORDER: usize = 19; // 16 << 19 = 8 MiB = one arena
pub const ARENA_SIZE: usize = MIN_ALLOC << MAX_ORDER;
const ARENA_PAGES: u64 = (ARENA_SIZE / 4096) as u64;
const MAX_ARENAS: usize = 64; // 512 MiB ceiling. generous.

/// Two pointers crammed into the first 16 bytes of every free block.
#[repr(C)]
struct FreeNode {
    next: *mut FreeNode,
    prev: *mut FreeNode,
}

struct HeapState {
    free_lists: [*mut FreeNode; MAX_ORDER + 1],
    arenas: [u64; MAX_ARENAS], // base VAs from SYS_MMAP
    arena_count: usize,
}

impl HeapState {
    const fn zeroed() -> Self {
        Self {
            free_lists: [ptr::null_mut(); MAX_ORDER + 1],
            arenas: [0; MAX_ARENAS],
            arena_count: 0,
        }
    }
}

// no SMP, no preemption — these are lies but safe lies.
unsafe impl Send for HeapState {}
unsafe impl Sync for HeapState {}

/// TAS spinlock. cooperative scheduling means this never actually spins.
struct SpinLock(AtomicBool);

impl SpinLock {
    const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    #[inline]
    fn lock(&self) {
        while self
            .0
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    #[inline]
    fn unlock(&self) {
        self.0.store(false, Ordering::Release);
    }
}

static HEAP: SpinLock = SpinLock::new();
static mut STATE: HeapState = HeapState::zeroed();

// all helpers below: caller holds HEAP lock. no exceptions.

unsafe fn list_push(state: &mut HeapState, order: usize, node: *mut FreeNode) {
    debug_assert!(!node.is_null());
    let old_head = state.free_lists[order];
    (*node).next = old_head;
    (*node).prev = ptr::null_mut();
    if !old_head.is_null() {
        (*old_head).prev = node;
    }
    state.free_lists[order] = node;
}

unsafe fn list_pop(state: &mut HeapState, order: usize) -> Option<*mut FreeNode> {
    let head = state.free_lists[order];
    if head.is_null() {
        return None;
    }
    let next = (*head).next;
    state.free_lists[order] = next;
    if !next.is_null() {
        (*next).prev = ptr::null_mut();
    }
    Some(head)
}

unsafe fn list_remove(state: &mut HeapState, order: usize, node: *mut FreeNode) {
    let prev = (*node).prev;
    let next = (*node).next;
    if !prev.is_null() {
        (*prev).next = next;
    } else {
        // node was the head
        state.free_lists[order] = next;
    }
    if !next.is_null() {
        (*next).prev = prev;
    }
    (*node).next = ptr::null_mut();
    (*node).prev = ptr::null_mut();
}

/// XOR buddy address. the one line of math that makes this whole thing work.
#[inline]
fn buddy_of(addr: u64, order: usize, arena_base: u64) -> u64 {
    let offset = addr - arena_base;
    let buddy_offset = offset ^ ((MIN_ALLOC as u64) << order);
    arena_base + buddy_offset
}

/// which arena owns this address? linear scan, sue me.
unsafe fn arena_of(state: &HeapState, addr: u64) -> Option<u64> {
    for i in 0..state.arena_count {
        let base = state.arenas[i];
        if addr >= base && addr < base + ARENA_SIZE as u64 {
            return Some(base);
        }
    }
    None
}

/// O(n) walk to check if buddy is free. yes it's slow. no we don't care yet.
unsafe fn is_free(state: &HeapState, order: usize, addr: u64) -> bool {
    let mut cur = state.free_lists[order];
    while !cur.is_null() {
        if cur as u64 == addr {
            return true;
        }
        cur = (*cur).next;
    }
    false
}

/// fresh 8 MiB arena from the kernel → one fat MAX_ORDER block.
unsafe fn add_arena(state: &mut HeapState, base: u64) {
    if state.arena_count >= MAX_ARENAS {
        return; // you have 512 MiB of heap. rethink your life choices.
    }
    state.arenas[state.arena_count] = base;
    state.arena_count += 1;
    let node = base as *mut FreeNode;
    (*node).next = ptr::null_mut();
    (*node).prev = ptr::null_mut();
    list_push(state, MAX_ORDER, node);
}

/// find a block, split it down, hand it over.
unsafe fn buddy_alloc(state: &mut HeapState, order: usize) -> Option<*mut u8> {
    let mut found = None;
    for k in order..=MAX_ORDER {
        if !state.free_lists[k].is_null() {
            found = Some(k);
            break;
        }
    }

    let found_order = match found {
        Some(k) => k,
        None => {
            // out of blocks — beg the kernel for more pages
            let va = crate::raw::syscall1(crate::raw::SYS_MMAP, ARENA_PAGES);
            if crate::is_error(va) {
                return None;
            }
            add_arena(state, va);

            // retry with the shiny new arena
            let mut refound = None;
            for k in order..=MAX_ORDER {
                if !state.free_lists[k].is_null() {
                    refound = Some(k);
                    break;
                }
            }
            refound?
        }
    };

    // split the block down to size
    let block = list_pop(state, found_order).unwrap();
    let mut cur_order = found_order;

    while cur_order > order {
        cur_order -= 1;
        // upper half goes back on the free list
        let buddy_addr = block as u64 + ((MIN_ALLOC as u64) << cur_order);
        let buddy = buddy_addr as *mut FreeNode;
        (*buddy).next = ptr::null_mut();
        (*buddy).prev = ptr::null_mut();
        list_push(state, cur_order, buddy);
    }

    Some(block as *mut u8)
}

/// return a block, coalesce with its buddy if possible. rinse, repeat.
unsafe fn buddy_free(state: &mut HeapState, ptr: *mut u8, order: usize) {
    let arena_base = match arena_of(state, ptr as u64) {
        Some(b) => b,
        None => return, // not our problem
    };

    let mut addr = ptr as u64;
    let mut cur_order = order;

    while cur_order < MAX_ORDER {
        let buddy_addr = buddy_of(addr, cur_order, arena_base);

        if buddy_addr < arena_base || buddy_addr >= arena_base + ARENA_SIZE as u64 {
            break;
        }

        if !is_free(state, cur_order, buddy_addr) {
            break;
        }

        let buddy_node = buddy_addr as *mut FreeNode;
        list_remove(state, cur_order, buddy_node);
        if buddy_addr < addr {
            addr = buddy_addr;
        }

        cur_order += 1;
    }

    let node = addr as *mut FreeNode;
    (*node).next = ptr::null_mut();
    (*node).prev = ptr::null_mut();
    list_push(state, cur_order, node);
}

/// layout → buddy order. must match between alloc and dealloc or you're hosed.
#[inline]
pub fn layout_to_order(layout: Layout) -> usize {
    let needed = layout.size().max(layout.align()).max(MIN_ALLOC);
    let p2 = needed.next_power_of_two();
    let log2_p2 = usize::BITS as usize - 1 - p2.leading_zeros() as usize;
    log2_p2.saturating_sub(MIN_ALLOC_SHIFT).min(MAX_ORDER)
}

pub struct BuddyHeap;

#[global_allocator]
pub static GLOBAL_HEAP: BuddyHeap = BuddyHeap;

unsafe impl GlobalAlloc for BuddyHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return NonNull::<u8>::dangling().as_ptr(); // ZST? here's a fake pointer, enjoy.
        }

        let order = layout_to_order(layout);
        HEAP.lock();
        let result = buddy_alloc(&mut STATE, order);
        HEAP.unlock();
        result.unwrap_or(ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() || layout.size() == 0 {
            return;
        }
        let order = layout_to_order(layout);
        HEAP.lock();
        buddy_free(&mut STATE, ptr, order);
        HEAP.unlock();
    }

    /// zero the whole block because recycled memory is dirty memory.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = self.alloc(layout);
        if !ptr.is_null() && layout.size() > 0 {
            let order = layout_to_order(layout);
            let block_size = MIN_ALLOC << order;
            ptr::write_bytes(ptr, 0, block_size);
        }
        ptr
    }

    /// alloc new, memcpy, free old. the classic.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size == 0 {
            self.dealloc(ptr, layout);
            return NonNull::<u8>::dangling().as_ptr();
        }
        let new_layout = match Layout::from_size_align(new_size, layout.align()) {
            Ok(l) => l,
            Err(_) => return ptr::null_mut(),
        };
        // same order? nothing to do. power-of-two sizing pays off here.
        if layout_to_order(layout) == layout_to_order(new_layout) {
            return ptr;
        }
        let new_ptr = self.alloc(new_layout);
        if !new_ptr.is_null() {
            let copy_size = layout.size().min(new_size);
            ptr::copy_nonoverlapping(ptr, new_ptr, copy_size);
            self.dealloc(ptr, layout);
        }
        new_ptr
    }
}
