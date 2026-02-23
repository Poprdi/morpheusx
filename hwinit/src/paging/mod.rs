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

    // UEFI / OVMF sets CR0.WP = 1 (Write Protect) and marks its own
    // page-table pages as read-only.  With WP=1, even Ring 0 code faults
    // when writing to a page whose PTE has R/W = 0.  Since we've adopted
    // these page tables as our own and need to freely modify entries (add
    // mappings, change caching flags, split huge pages), clear CR0.WP.
    let cr0: u64;
    core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
    if cr0 & (1u64 << 16) != 0 {
        core::arch::asm!(
            "mov cr0, {}",
            in(reg) cr0 & !(1u64 << 16),
            options(nomem, nostack),
        );
        puts("[PAGING] cleared CR0.WP — kernel owns page tables\n");
    }

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
    KERNEL_PT
        .as_ref()
        .expect("kernel page table not initialized")
}

/// Borrow the kernel `PageTableManager` (mutable).
///
/// # Safety
/// Must only be called after `init_kernel_page_table()`.  Caller is
/// responsible for serializing access (single-threaded kernel is fine).
pub unsafe fn kernel_page_table_mut() -> &'static mut PageTableManager {
    KERNEL_PT
        .as_mut()
        .expect("kernel page table not initialized")
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

/// Split any huge pages along the path to `virt` in the kernel page table
/// so that a subsequent `kmap_4k()` call will succeed.
///
/// # Safety
/// See [`PageTableManager::ensure_4k_mappable`].
pub unsafe fn kensure_4k(virt: u64) -> Result<(), &'static str> {
    kernel_page_table_mut().ensure_4k_mappable(virt)
}

/// Identity-map a physical MMIO region with UC flags in the kernel page table.
///
/// Handles all cases: existing huge pages (sets UC bits), existing 4K pages
/// (sets UC bits), and unmapped regions (creates new identity-mapped entries).
///
/// # Safety
/// See [`PageTableManager::map_mmio`].
pub unsafe fn kmap_mmio(phys: u64, size: u64) -> Result<(), &'static str> {
    kernel_page_table_mut().map_mmio(phys, size)
}

/// Mark the leaf page table entry covering `virt` as uncacheable (UC).
///
/// Works with 1 GiB, 2 MiB, or 4 KiB pages.  Ideal for PCI MMIO BAR
/// regions that UEFI identity-mapped with Write-Back caching.
///
/// # Safety
/// See [`PageTableManager::mark_uncacheable`].
pub unsafe fn kmark_uncacheable(virt: u64) -> Result<(), &'static str> {
    kernel_page_table_mut().mark_uncacheable(virt)
}

// ═══════════════════════════════════════════════════════════════════════════
// PAGE TABLE RESERVATION
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum number of page-table pages we expect to encounter.
///
/// UEFI/OVMF identity-maps up to ~12 GB of RAM.  With 2 MiB huge pages the
/// page table tree is shallow:
///   - 1 PML4    (always)
///   - up to ~6  PDPT pages (one per 512 GiB)
///   - up to ~24 PD pages   (one per 1 GiB)
///   - PT pages only if 4 KiB region exists (firmware code, MMIO, etc.)
///
/// 512 entries is vastly more than needed but fits comfortably on the stack.
const MAX_PT_PAGES: usize = 512;

/// Walk the active CR3 page table hierarchy and collect the physical
/// addresses of **every** page that is itself a page table page (PML4,
/// PDPT, PD, PT).
///
/// This is used very early — before `init_kernel_page_table()` — so it
/// does NOT depend on any `PageTableManager` state.  It reads CR3 directly
/// and interprets identity-mapped pointers.
///
/// Returns `(pages_array, count)`.
///
/// # Safety
/// - Must run in 64-bit long mode with paging active.
/// - Physical == virtual (identity-mapped).
pub unsafe fn collect_page_table_pages() -> ([u64; MAX_PT_PAGES], usize) {
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
    let pml4_phys = cr3 & !0xFFFu64;

    let mut pages = [0u64; MAX_PT_PAGES];
    let mut count = 0usize;

    // Helper closure (inlined) to add a page if not already seen.
    macro_rules! add_page {
        ($phys:expr) => {
            let p = $phys;
            let mut seen = false;
            for j in 0..count {
                if pages[j] == p {
                    seen = true;
                    break;
                }
            }
            if !seen && count < MAX_PT_PAGES {
                pages[count] = p;
                count += 1;
            }
        };
    }

    // PML4 itself
    add_page!(pml4_phys);

    let pml4 = pml4_phys as *const u64;
    for i in 0..512usize {
        let e1 = *pml4.add(i);
        if e1 & 1 == 0 {
            continue;
        } // not present
        let pdpt_phys = e1 & 0x000F_FFFF_FFFF_F000;
        add_page!(pdpt_phys);

        let pdpt = pdpt_phys as *const u64;
        for j in 0..512usize {
            let e2 = *pdpt.add(j);
            if e2 & 1 == 0 {
                continue;
            } // not present
            if e2 & (1 << 7) != 0 {
                continue;
            } // 1 GiB huge page — no sub-table
            let pd_phys = e2 & 0x000F_FFFF_FFFF_F000;
            add_page!(pd_phys);

            let pd = pd_phys as *const u64;
            for k in 0..512usize {
                let e3 = *pd.add(k);
                if e3 & 1 == 0 {
                    continue;
                } // not present
                if e3 & (1 << 7) != 0 {
                    continue;
                } // 2 MiB huge page
                let pt_phys = e3 & 0x000F_FFFF_FFFF_F000;
                add_page!(pt_phys);
            }
        }
    }

    (pages, count)
}

/// Reserve every page that the currently-active CR3 page table hierarchy
/// uses, so that the memory registry never hands them out.
///
/// Must be called after `init_global_registry()` but **before** any
/// `allocate_pages()` calls that use `MaxAddress` or `AnyPages` with
/// the free-list path — otherwise the allocator might return a page that
/// is actively part of the live page table tree.
///
/// # Safety
/// - Identity-mapped, long-mode, paging active.
/// - Memory registry must be initialized.
pub unsafe fn reserve_page_table_pages() -> usize {
    use crate::memory::{global_registry_mut, AllocateType, MemoryType};
    use crate::serial::{put_hex32, put_hex64};

    let (pt_pages, pt_count) = collect_page_table_pages();

    puts("[PAGING] reserving ");
    put_hex32(pt_count as u32);
    puts(" page-table pages from allocator\n");

    let registry = global_registry_mut();
    let mut reserved = 0usize;

    for &phys in pt_pages.iter().take(pt_count) {
        match registry.allocate_pages(
            AllocateType::Address(phys),
            MemoryType::AllocatedPageTable,
            1,
        ) {
            Ok(_) => {
                reserved += 1;
            }
            Err(_) => {
                // Page might already be in a non-free region (e.g. RuntimeServices).
                // That's fine — it just means the allocator can't hand it out anyway.
                puts("[PAGING]   page ");
                put_hex64(phys);
                puts(" not in free list (already reserved)\n");
            }
        }
    }

    puts("[PAGING] reserved ");
    put_hex32(reserved as u32);
    puts(" / ");
    put_hex32(pt_count as u32);
    puts(" page-table pages\n");

    reserved
}
