//! Hybrid global allocator.
//!
//! Pre-EBS: UEFI allocate_pool / free_pool.
//! Post-EBS: 4 MB .bss primary heap + on-demand overflow from MemoryRegistry,
//! growing in 16 MB chunks up to 256 MB. Dealloc routes by address range.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use linked_list_allocator::Heap;
use spin::Mutex;

const EFI_LOADER_DATA: usize = 2;

/// .bss bootstrap heap; lives until registry init.
const PRIMARY_HEAP_SIZE: usize = 4 * 1024 * 1024;
const OVERFLOW_GROW_CHUNK: usize = 16 * 1024 * 1024;
const OVERFLOW_MAX_SIZE: usize = 512 * 1024 * 1024;

/// Pre-EBS only.
static BOOT_SERVICES: AtomicPtr<()> = AtomicPtr::new(ptr::null_mut());
static POST_EBS: AtomicBool = AtomicBool::new(false);

#[repr(C, align(4096))]
struct AlignedHeapBuffer([u8; PRIMARY_HEAP_SIZE]);

static mut HEAP_BUFFER: AlignedHeapBuffer = AlignedHeapBuffer([0u8; PRIMARY_HEAP_SIZE]);

static PRIMARY_HEAP: Mutex<Heap> = Mutex::new(Heap::empty());
/// For dealloc range check.
static PRIMARY_BASE: AtomicU64 = AtomicU64::new(0);

struct OverflowState {
    heap: Heap,
    base: u64,
    size: usize,
}

static OVERFLOW_HEAP: Mutex<Option<OverflowState>> = Mutex::new(None);
/// 0 == not yet initialized.
static OVERFLOW_BASE: AtomicU64 = AtomicU64::new(0);
static OVERFLOW_SIZE: AtomicUsize = AtomicUsize::new(0);

/// Call once at efi_main entry.
pub fn set_boot_services(bs: *const crate::BootServices) {
    BOOT_SERVICES.store(bs as *mut (), Ordering::SeqCst);
}

/// # Safety
/// Call exactly once, right after ExitBootServices returns.
pub unsafe fn switch_to_post_ebs() {
    let heap_start = HEAP_BUFFER.0.as_mut_ptr();
    PRIMARY_HEAP.lock().init(heap_start, PRIMARY_HEAP_SIZE);
    PRIMARY_BASE.store(heap_start as u64, Ordering::SeqCst);

    BOOT_SERVICES.store(ptr::null_mut(), Ordering::SeqCst);

    POST_EBS.store(true, Ordering::SeqCst);
}

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

/// primary → overflow (init/grow on demand).
unsafe fn post_ebs_alloc(layout: Layout) -> *mut u8 {
    {
        let mut h = PRIMARY_HEAP.lock();
        if let Ok(nn) = h.allocate_first_fit(layout) {
            return nn.as_ptr();
        }
    }

    let mut guard = OVERFLOW_HEAP.lock();

    if guard.is_none() {
        match try_init_overflow() {
            Some(state) => *guard = Some(state),
            None => return ptr::null_mut(),
        }
    }

    let state = guard.as_mut().unwrap();

    if let Ok(nn) = state.heap.allocate_first_fit(layout) {
        return nn.as_ptr();
    }

    if try_grow_overflow(state, layout.size()) {
        state
            .heap
            .allocate_first_fit(layout)
            .map(|nn| nn.as_ptr())
            .unwrap_or(ptr::null_mut())
    } else {
        ptr::null_mut()
    }
}

unsafe fn post_ebs_dealloc(ptr: *mut u8, layout: Layout) {
    let addr = ptr as u64;

    let pb = PRIMARY_BASE.load(Ordering::SeqCst);
    if addr >= pb && addr < pb + PRIMARY_HEAP_SIZE as u64 {
        if let Some(nn) = NonNull::new(ptr) {
            PRIMARY_HEAP.lock().deallocate(nn, layout);
        }
        return;
    }

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

    morpheus_hal_x86_64::serial::puts("[ALLOC] WARN: dealloc ptr ");
    morpheus_hal_x86_64::serial::put_hex64(addr);
    morpheus_hal_x86_64::serial::puts(" not in any heap\n");
}

unsafe fn try_init_overflow() -> Option<OverflowState> {
    if !morpheus_hal_x86_64::memory::is_registry_initialized() {
        return None;
    }

    let mut registry = morpheus_hal_x86_64::memory::global_registry_mut();
    let pages = (OVERFLOW_GROW_CHUNK as u64).div_ceil(morpheus_hal_x86_64::memory::PAGE_SIZE);

    match registry.allocate_pages(
        morpheus_hal_x86_64::memory::AllocateType::AnyPages,
        morpheus_hal_x86_64::memory::MemoryType::AllocatedHeap,
        pages,
    ) {
        Ok(base) => {
            let mut heap = Heap::empty();
            heap.init(base as *mut u8, OVERFLOW_GROW_CHUNK);

            OVERFLOW_BASE.store(base, Ordering::SeqCst);
            OVERFLOW_SIZE.store(OVERFLOW_GROW_CHUNK, Ordering::SeqCst);

            Some(OverflowState {
                heap,
                base,
                size: OVERFLOW_GROW_CHUNK,
            })
        },
        Err(_) => {
            morpheus_hal_x86_64::serial::puts(
                "[ALLOC] ERROR: MemoryRegistry refused overflow allocation!\n",
            );
            None
        },
    }
}

/// Grow overflow heap by `OVERFLOW_GROW_CHUNK` via contiguous page alloc.
unsafe fn try_grow_overflow(state: &mut OverflowState, _needed: usize) -> bool {
    if state.size >= OVERFLOW_MAX_SIZE {
        morpheus_hal_x86_64::serial::puts("[ALLOC] Overflow heap at hard limit (256 MB)\n");
        return false;
    }

    if !morpheus_hal_x86_64::memory::is_registry_initialized() {
        return false;
    }

    let grow = OVERFLOW_GROW_CHUNK.min(OVERFLOW_MAX_SIZE - state.size);
    let pages = (grow as u64).div_ceil(morpheus_hal_x86_64::memory::PAGE_SIZE);

    // Pages must be contiguous with current heap end so Heap::extend() can merge.
    let extend_addr = state.base + state.size as u64;

    let mut registry = morpheus_hal_x86_64::memory::global_registry_mut();
    match registry.allocate_pages(
        morpheus_hal_x86_64::memory::AllocateType::Address(extend_addr),
        morpheus_hal_x86_64::memory::MemoryType::AllocatedHeap,
        pages,
    ) {
        Ok(_) => {
            state.heap.extend(grow);
            state.size += grow;
            OVERFLOW_SIZE.store(state.size, Ordering::SeqCst);

            true
        },
        Err(_) => {
            // Adjacent pages taken; we don't support disjoint heap regions.
            morpheus_hal_x86_64::serial::puts(
                "[ALLOC] Overflow grow failed: address not free in registry\n",
            );
            false
        },
    }
}

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
        if status == 0 {
            buffer
        } else {
            ptr::null_mut()
        }
    } else {
        // Overallocate; stash original pointer just before aligned addr.
        let total_size = size + align + core::mem::size_of::<usize>();
        let mut buffer: *mut u8 = ptr::null_mut();
        let status = (bs.allocate_pool)(EFI_LOADER_DATA, total_size, &mut buffer);
        if status != 0 {
            return ptr::null_mut();
        }

        let raw_addr = buffer as usize;
        let aligned_addr = (raw_addr + core::mem::size_of::<usize>() + align - 1) & !(align - 1);
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

#[global_allocator]
static ALLOCATOR: HybridAllocator = HybridAllocator;
