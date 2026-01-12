//! Hybrid Global Allocator
//!
//! Pre-EBS: Uses UEFI's allocate_pool/free_pool
//! Post-EBS: Uses linked_list_allocator with static buffer
//!
//! Call `switch_to_post_ebs()` after ExitBootServices to switch modes.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use linked_list_allocator::Heap;
use spin::Mutex;

/// UEFI Boot Services pointer - set by efi_main
static BOOT_SERVICES: AtomicPtr<()> = AtomicPtr::new(ptr::null_mut());

/// Track if we're post-EBS (using linked_list_allocator)
static POST_EBS: AtomicBool = AtomicBool::new(false);

/// EFI_LOADER_DATA memory type
const EFI_LOADER_DATA: usize = 2;

/// Post-EBS heap size: 4MB
const HEAP_SIZE: usize = 4 * 1024 * 1024;

/// Page-aligned heap buffer for post-EBS
#[repr(C, align(4096))]
struct AlignedHeapBuffer([u8; HEAP_SIZE]);

/// Static heap buffer - lives in .bss, zero-initialized
static mut HEAP_BUFFER: AlignedHeapBuffer = AlignedHeapBuffer([0u8; HEAP_SIZE]);

/// Locked heap for post-EBS
static POST_EBS_HEAP: Mutex<Heap> = Mutex::new(Heap::empty());

/// Set the boot services pointer (call once at start of efi_main)
pub fn set_boot_services(bs: *const crate::BootServices) {
    BOOT_SERVICES.store(bs as *mut (), Ordering::SeqCst);
}

/// Switch to post-EBS allocator (call after ExitBootServices)
///
/// # Safety
/// Must only be called once, after ExitBootServices
pub unsafe fn switch_to_post_ebs() {
    // Initialize the linked_list_allocator heap
    let heap_start = HEAP_BUFFER.0.as_mut_ptr();
    POST_EBS_HEAP.lock().init(heap_start, HEAP_SIZE);

    // Clear boot services pointer (no longer valid)
    BOOT_SERVICES.store(ptr::null_mut(), Ordering::SeqCst);

    // Switch to post-EBS mode
    POST_EBS.store(true, Ordering::SeqCst);
}

/// Hybrid allocator
pub struct HybridAllocator;

unsafe impl GlobalAlloc for HybridAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if POST_EBS.load(Ordering::SeqCst) {
            // Post-EBS: use linked_list_allocator
            POST_EBS_HEAP
                .lock()
                .allocate_first_fit(layout)
                .map(|nn| nn.as_ptr())
                .unwrap_or(ptr::null_mut())
        } else {
            // Pre-EBS: use UEFI allocate_pool
            alloc_uefi(layout)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }

        if POST_EBS.load(Ordering::SeqCst) {
            // Post-EBS: use linked_list_allocator
            if let Some(nn) = NonNull::new(ptr) {
                POST_EBS_HEAP.lock().deallocate(nn, layout);
            }
        } else {
            // Pre-EBS: use UEFI free_pool
            dealloc_uefi(ptr, layout);
        }
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
        // Simple case - UEFI alignment is sufficient
        let mut buffer: *mut u8 = ptr::null_mut();
        let status = (bs.allocate_pool)(EFI_LOADER_DATA, size, &mut buffer);
        if status == 0 {
            buffer
        } else {
            ptr::null_mut()
        }
    } else {
        // Need to over-allocate for alignment
        let total_size = size + align + core::mem::size_of::<usize>();
        let mut buffer: *mut u8 = ptr::null_mut();
        let status = (bs.allocate_pool)(EFI_LOADER_DATA, total_size, &mut buffer);
        if status != 0 {
            return ptr::null_mut();
        }

        let raw_addr = buffer as usize;
        let aligned_addr = (raw_addr + core::mem::size_of::<usize>() + align - 1) & !(align - 1);
        let original_ptr_location = (aligned_addr - core::mem::size_of::<usize>()) as *mut usize;
        *original_ptr_location = raw_addr;

        aligned_addr as *mut u8
    }
}

unsafe fn dealloc_uefi(ptr: *mut u8, layout: Layout) {
    let bs_ptr = BOOT_SERVICES.load(Ordering::SeqCst);
    if bs_ptr.is_null() {
        return;
    }

    let bs = &*(bs_ptr as *const crate::BootServices);
    let align = layout.align();

    if align <= 8 {
        let _ = (bs.free_pool)(ptr);
    } else {
        let original_ptr_location = (ptr as usize - core::mem::size_of::<usize>()) as *mut usize;
        let original_ptr = *original_ptr_location as *mut u8;
        let _ = (bs.free_pool)(original_ptr);
    }
}

#[global_allocator]
static ALLOCATOR: HybridAllocator = HybridAllocator;
