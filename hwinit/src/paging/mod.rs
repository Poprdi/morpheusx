//! x86-64 Paging Manager
//!
//! Provides a safe-ish, `no_std` interface for the 4-level x86-64 page table
//! tree.  Currently operates in *identity-mapped* mode (physical == virtual),
//! which is how UEFI leaves the CPU.  Process isolation (per-process PML4s)
//! is built on top of this in Phase 3+.
//!
//! # Usage
//!
//! After `platform_init_selfcontained()` completes (i.e., MemoryRegistry and
//! the global heap are both up), call:
//!
//! ```ignore
//! unsafe {
//!     // Adopt the UEFI page tables as the kernel's own.
//!     morpheus_hwinit::paging::init_kernel_page_table();
//!
//!     // Map/unmap additional pages:
//!     let mut pt = morpheus_hwinit::paging::kernel_page_table();
//!     pt.map_4k(virt, phys, PageFlags::KERNEL_RW)?;
//! }
//! ```

pub mod entry;
pub mod table;

pub use entry::{PageFlags, PageTable, PageTableEntry};
pub use table::{MappedPageSize, PageTableManager, VirtAddr};

use crate::serial::puts;

// ═══════════════════════════════════════════════════════════════════════════
// GLOBAL KERNEL PAGE TABLE
// ═══════════════════════════════════════════════════════════════════════════

/// Singleton kernel `PageTableManager`.
///
/// Initialized by `init_kernel_page_table()`.  Access via
/// `kernel_page_table()` / `kernel_page_table_mut()`.
static mut KERNEL_PT: Option<PageTableManager> = None;
static mut PAGING_INITIALIZED: bool = false;

/// Initialize the kernel page table manager by reading the current CR3.
///
/// Must be called once, after MemoryRegistry is ready (Phase 1 of hwinit).
///
/// # Safety
/// - Must run in long mode with paging active.
/// - Must be called exactly once.
pub unsafe fn init_kernel_page_table() {
    if PAGING_INITIALIZED {
        puts("[PAGING] already initialized — skipping\n");
        return;
    }

    let mgr = PageTableManager::from_cr3();
    puts("[PAGING] adopted UEFI PML4 @ ");
    crate::serial::put_hex64(mgr.pml4_phys);
    puts("\n");

    KERNEL_PT = Some(mgr);
    PAGING_INITIALIZED = true;
}

/// Returns true if the kernel page table has been initialized.
pub fn is_paging_initialized() -> bool {
    unsafe { PAGING_INITIALIZED }
}

/// Borrow the kernel `PageTableManager` (immutable).
///
/// # Safety
/// Must only be called after `init_kernel_page_table()`.
pub unsafe fn kernel_page_table() -> &'static PageTableManager {
    KERNEL_PT.as_ref().expect("kernel page table not initialized")
}

/// Borrow the kernel `PageTableManager` (mutable).
///
/// # Safety
/// Must only be called after `init_kernel_page_table()`.  Caller is
/// responsible for serializing access (single-threaded kernel is fine).
pub unsafe fn kernel_page_table_mut() -> &'static mut PageTableManager {
    KERNEL_PT.as_mut().expect("kernel page table not initialized")
}

// ═══════════════════════════════════════════════════════════════════════════
// CONVENIENCE WRAPPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Map a single 4 KiB page in the kernel page table.
///
/// # Safety
/// See [`PageTableManager::map_4k`].
pub unsafe fn kmap_4k(virt: u64, phys: u64, flags: PageFlags) -> Result<(), &'static str> {
    kernel_page_table_mut().map_4k(virt, phys, flags)
}

/// Map a 2 MiB huge page in the kernel page table.
///
/// # Safety
/// See [`PageTableManager::map_2m`].
pub unsafe fn kmap_2m(virt: u64, phys: u64, flags: PageFlags) -> Result<(), &'static str> {
    kernel_page_table_mut().map_2m(virt, phys, flags)
}

/// Unmap a 4 KiB page from the kernel page table.
///
/// # Safety
/// See [`PageTableManager::unmap_4k`].
pub unsafe fn kunmap_4k(virt: u64) -> Result<(), &'static str> {
    kernel_page_table_mut().unmap_4k(virt)
}

/// Translate a virtual address to physical using the kernel page table.
///
/// Returns `None` if the address is not mapped.
///
/// # Safety
/// See [`PageTableManager::translate`].
pub unsafe fn kvirt_to_phys(virt: u64) -> Option<u64> {
    kernel_page_table().translate(virt)
}
