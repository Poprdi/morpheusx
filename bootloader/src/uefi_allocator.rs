//! Hybrid Global Allocator
//!
//! Pre-EBS:  Uses UEFI's allocate_pool / free_pool.
//! Post-EBS: Uses a two-region linked_list_allocator scheme:
//!   - Primary heap  — 4 MB static .bss bootstrap buffer (no registry needed)
//!   - Overflow heap — allocated on-demand from MemoryRegistry when primary OOM
//!     The overflow heap can grow in 16 MB chunks up to 256 MB.
//!
//! ## Flow
//!
//! ```text
//! alloc()  ─────►  primary heap  ─────► OK
//!                     │ OOM
//!                     ▼
//!              overflow heap (init/grow via MemoryRegistry)
//!                     │ OK
//!                     ▼
//!                  return ptr      (or null if registry also fails)
//!
//! dealloc() ──► check ptr range ──► route to correct Heap
//! ```
//!
//! ## Boot ordering
//!
//! Between ExitBootServices and MemoryRegistry init (hwinit Phase 1), only the
//! 4 MB primary heap is available.  Once `morpheus_hwinit::is_registry_initialized()`
//! returns true, overflow can be requested transparently on the next OOM.
//!
//! Call `switch_to_post_ebs()` immediately after ExitBootServices.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use linked_list_allocator::Heap;
use spin::Mutex;

// ─────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────

/// UEFI EFI_LOADER_DATA memory type constant.
const EFI_LOADER_DATA: usize = 2;

/// Primary (bootstrap) heap in .bss: 4 MB.  Must survive until registry init.
const PRIMARY_HEAP_SIZE: usize = 4 * 1024 * 1024;

/// Size of each overflow chunk requested from MemoryRegistry.
const OVERFLOW_GROW_CHUNK: usize = 16 * 1024 * 1024;

/// Hard cap on overflow heap size.
const OVERFLOW_MAX_SIZE: usize = 256 * 1024 * 1024;

// ─────────────────────────────────────────────────────────────────────────
// Static storage
// ─────────────────────────────────────────────────────────────────────────

/// UEFI BootServices pointer (valid pre-EBS only).
static BOOT_SERVICES: AtomicPtr<()> = AtomicPtr::new(ptr::null_mut());

/// True once switch_to_post_ebs() has been called.
static POST_EBS: AtomicBool = AtomicBool::new(false);

/// Page-aligned primary heap buffer — lives in .bss, zero-initialised.
#[repr(C, align(4096))]
struct AlignedHeapBuffer([u8; PRIMARY_HEAP_SIZE]);

static mut HEAP_BUFFER: AlignedHeapBuffer = AlignedHeapBuffer([0u8; PRIMARY_HEAP_SIZE]);

/// Primary heap — backed by HEAP_BUFFER.
static PRIMARY_HEAP: Mutex<Heap> = Mutex::new(Heap::empty());

/// Physical start of primary heap (set at switch_to_post_ebs time).
/// Used for dealloc routing.
static PRIMARY_BASE: AtomicU64 = AtomicU64::new(0);

// ─────────────────────────────────────────────────────────────────────────
// Overflow heap
// ─────────────────────────────────────────────────────────────────────────

/// State for the dynamically-allocated overflow heap.
struct OverflowState {
    heap: Heap,
    /// Physical base address of the current heap region.
    base: u64,
    /// Total committed size (may grow by OVERFLOW_GROW_CHUNK).
    size: usize,
}

/// The overflow heap — None until first primary OOM that can be satisfied.
static OVERFLOW_HEAP: Mutex<Option<OverflowState>> = Mutex::new(None);

/// Physical base of overflow heap (for dealloc routing; 0 = not yet init).
static OVERFLOW_BASE: AtomicU64 = AtomicU64::new(0);

/// Current committed size of overflow heap (for dealloc routing).
static OVERFLOW_SIZE: AtomicUsize = AtomicUsize::new(0);

// ─────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────

/// Register the UEFI BootServices pointer.  Call once at efi_main entry.
pub fn set_boot_services(bs: *const crate::BootServices) {
    BOOT_SERVICES.store(bs as *mut (), Ordering::SeqCst);
}

/// Switch to post-EBS mode.  Call immediately after ExitBootServices.
///
/// # Safety
/// Must be called exactly once, after ExitBootServices returns.
pub unsafe fn switch_to_post_ebs() {
    let heap_start = HEAP_BUFFER.0.as_mut_ptr();
    PRIMARY_HEAP.lock().init(heap_start, PRIMARY_HEAP_SIZE);
    PRIMARY_BASE.store(heap_start as u64, Ordering::SeqCst);

    // Invalidate BootServices (no longer valid).
    BOOT_SERVICES.store(ptr::null_mut(), Ordering::SeqCst);

    POST_EBS.store(true, Ordering::SeqCst);
}

// ─────────────────────────────────────────────────────────────────────────
// GlobalAlloc implementation
// ─────────────────────────────────────────────────────────────────────────

/// The hybrid allocator type registered as #[global_allocator].
pub struct HybridAllocator;

unsafe impl GlobalAlloc for HybridAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if POST_EBS.load(Ordering::SeqCst) {
            post_ebs_alloc(layout)
        } else {
            alloc_uefi(layout)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }
        if POST_EBS.load(Ordering::SeqCst) {
            post_ebs_dealloc(ptr, layout);
        } else {
            dealloc_uefi(ptr, layout);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Post-EBS allocation path
// ─────────────────────────────────────────────────────────────────────────

/// Post-EBS allocation: primary → overflow (init / grow on demand).
unsafe fn post_ebs_alloc(layout: Layout) -> *mut u8 {
    // ── 1. Try primary heap ──────────────────────────────────────────────
    {
        let mut h = PRIMARY_HEAP.lock();
        if let Ok(nn) = h.allocate_first_fit(layout) {
            return nn.as_ptr();
        }
    }

    // ── 2. Primary OOM — use / initialise overflow heap ──────────────────
    let mut guard = OVERFLOW_HEAP.lock();

    if guard.is_none() {
        // First OOM: try to allocate the first overflow chunk.
        match try_init_overflow() {
            Some(state) => *guard = Some(state),
            None => return ptr::null_mut(),
        }
    }

    let state = guard.as_mut().unwrap();

    // ── 3. Try allocation in overflow ────────────────────────────────────
    if let Ok(nn) = state.heap.allocate_first_fit(layout) {
        return nn.as_ptr();
    }

    // ── 4. Overflow OOM — try to grow it ────────────────────────────────
    if try_grow_overflow(state, layout.size()) {
        state.heap
            .allocate_first_fit(layout)
            .map(|nn| nn.as_ptr())
            .unwrap_or(ptr::null_mut())
    } else {
        ptr::null_mut()
    }
}

/// Post-EBS dealloc: route pointer to whichever heap owns it.
unsafe fn post_ebs_dealloc(ptr: *mut u8, layout: Layout) {
    let addr = ptr as u64;

    // Check primary heap range.
    let pb = PRIMARY_BASE.load(Ordering::SeqCst);
    if addr >= pb && addr < pb + PRIMARY_HEAP_SIZE as u64 {
        if let Some(nn) = NonNull::new(ptr) {
            PRIMARY_HEAP.lock().deallocate(nn, layout);
        }
        return;
    }

    // Check overflow heap range.
    let ob = OVERFLOW_BASE.load(Ordering::SeqCst);
    let os = OVERFLOW_SIZE.load(Ordering::SeqCst) as u64;
    if ob != 0 && addr >= ob && addr < ob + os {
        let mut guard = OVERFLOW_HEAP.lock();
        if let Some(state) = guard.as_mut() {
            if let Some(nn) = NonNull::new(ptr) {
                state.heap.deallocate(nn, layout);
            }
        }
        return;
    }

    // Pointer not in any known heap — log and move on (can't panic safely).
    morpheus_hwinit::serial::puts("[ALLOC] WARN: dealloc ptr ");
    morpheus_hwinit::serial::put_hex64(addr);
    morpheus_hwinit::serial::puts(" not in any heap\n");
}

// ─────────────────────────────────────────────────────────────────────────
// Overflow heap management (MemoryRegistry backed)
// ─────────────────────────────────────────────────────────────────────────

/// Allocate the first overflow chunk from MemoryRegistry.
/// Returns None if registry is not yet ready or allocation fails.
unsafe fn try_init_overflow() -> Option<OverflowState> {
    if !morpheus_hwinit::is_registry_initialized() {
        morpheus_hwinit::serial::puts(
            "[ALLOC] Primary OOM — registry not ready, cannot grow\n"
        );
        return None;
    }

    morpheus_hwinit::serial::puts(
        "[ALLOC] Primary heap OOM — allocating 16 MB overflow from registry\n"
    );

    let registry = morpheus_hwinit::global_registry_mut();
    let pages = (OVERFLOW_GROW_CHUNK as u64 + morpheus_hwinit::PAGE_SIZE - 1)
        / morpheus_hwinit::PAGE_SIZE;

    match registry.allocate_pages(
        morpheus_hwinit::AllocateType::AnyPages,
        morpheus_hwinit::MemoryType::AllocatedHeap,
        pages,
    ) {
        Ok(base) => {
            let mut heap = Heap::empty();
            heap.init(base as *mut u8, OVERFLOW_GROW_CHUNK);

            OVERFLOW_BASE.store(base, Ordering::SeqCst);
            OVERFLOW_SIZE.store(OVERFLOW_GROW_CHUNK, Ordering::SeqCst);

            morpheus_hwinit::serial::puts("[ALLOC] Overflow heap @ ");
            morpheus_hwinit::serial::put_hex64(base);
            morpheus_hwinit::serial::puts(", 16 MB\n");

            Some(OverflowState {
                heap,
                base,
                size: OVERFLOW_GROW_CHUNK,
            })
        }
        Err(_) => {
            morpheus_hwinit::serial::puts(
                "[ALLOC] ERROR: MemoryRegistry refused overflow allocation!\n"
            );
            None
        }
    }
}

/// Grow the overflow heap by OVERFLOW_GROW_CHUNK via contiguous page allocation.
/// Returns true on success.
unsafe fn try_grow_overflow(state: &mut OverflowState, _needed: usize) -> bool {
    if state.size >= OVERFLOW_MAX_SIZE {
        morpheus_hwinit::serial::puts("[ALLOC] Overflow heap at hard limit (256 MB)\n");
        return false;
    }

    if !morpheus_hwinit::is_registry_initialized() {
        return false;
    }

    let grow = OVERFLOW_GROW_CHUNK.min(OVERFLOW_MAX_SIZE - state.size);
    let pages = (grow as u64 + morpheus_hwinit::PAGE_SIZE - 1) / morpheus_hwinit::PAGE_SIZE;

    // We need the new pages to be contiguous with the current heap end so
    // linked_list_allocator::Heap::extend() can merge them into the free list.
    let extend_addr = state.base + state.size as u64;

    let registry = morpheus_hwinit::global_registry_mut();
    match registry.allocate_pages(
        morpheus_hwinit::AllocateType::Address(extend_addr),
        morpheus_hwinit::MemoryType::AllocatedHeap,
        pages,
    ) {
        Ok(_) => {
            state.heap.extend(grow);
            state.size += grow;
            OVERFLOW_SIZE.store(state.size, Ordering::SeqCst);

            morpheus_hwinit::serial::puts("[ALLOC] Overflow heap grew +16 MB, total = ");
            morpheus_hwinit::serial::put_hex32(state.size as u32);
            morpheus_hwinit::serial::puts(" bytes\n");
            true
        }
        Err(_) => {
            // Memory at that exact address is taken — can't extend without
            // a second disjoint heap region (not supported here).
            morpheus_hwinit::serial::puts(
                "[ALLOC] Overflow grow failed: address not free in registry\n"
            );
            false
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Pre-EBS UEFI allocation path
// ─────────────────────────────────────────────────────────────────────────

unsafe fn alloc_uefi(layout: Layout) -> *mut u8 {
    let bs_ptr = BOOT_SERVICES.load(Ordering::SeqCst);
    if bs_ptr.is_null() {
        return ptr::null_mut();
    }

    let bs = &*(bs_ptr as *const crate::BootServices);
    let align = layout.align();
    let size = layout.size();

    if align <= 8 {
        let mut buffer: *mut u8 = ptr::null_mut();
        let status = (bs.allocate_pool)(EFI_LOADER_DATA, size, &mut buffer);
        if status == 0 { buffer } else { ptr::null_mut() }
    } else {
        // Over-allocate and store original pointer just before aligned addr.
        let total_size = size + align + core::mem::size_of::<usize>();
        let mut buffer: *mut u8 = ptr::null_mut();
        let status = (bs.allocate_pool)(EFI_LOADER_DATA, total_size, &mut buffer);
        if status != 0 {
            return ptr::null_mut();
        }

        let raw_addr = buffer as usize;
        let aligned_addr =
            (raw_addr + core::mem::size_of::<usize>() + align - 1) & !(align - 1);
        let header = (aligned_addr - core::mem::size_of::<usize>()) as *mut usize;
        *header = raw_addr;
        aligned_addr as *mut u8
    }
}

unsafe fn dealloc_uefi(ptr: *mut u8, layout: Layout) {
    let bs_ptr = BOOT_SERVICES.load(Ordering::SeqCst);
    if bs_ptr.is_null() {
        return;
    }

    let bs = &*(bs_ptr as *const crate::BootServices);

    if layout.align() <= 8 {
        let _ = (bs.free_pool)(ptr);
    } else {
        let header = (ptr as usize - core::mem::size_of::<usize>()) as *mut usize;
        let original = *header as *mut u8;
        let _ = (bs.free_pool)(original);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Registration
// ─────────────────────────────────────────────────────────────────────────

#[global_allocator]
static ALLOCATOR: HybridAllocator = HybridAllocator;


