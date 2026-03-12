//! 4-level x86-64 page tables. Identity-mapped (VA=PA) post-UEFI.
//! Per-process PML4 clones for user isolation.

pub mod entry;
pub mod table;

pub use entry::{PageFlags, PageTable, PageTableEntry};
pub use table::{MappedPageSize, PageTableManager, VirtAddr};

use crate::serial::{log_info, log_ok, log_warn};
use crate::sync::SpinLock;

// kernel page table singleton — SMP-safe via SpinLock
static KERNEL_PT: SpinLock<Option<PageTableManager>> = SpinLock::new(None);
static PAGING_INITIALIZED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Initialize the kernel page table manager by reading the current CR3.
///
/// Must be called once, after MemoryRegistry is ready (Phase 1 of hwinit).
///
/// # Safety
/// - Must run in long mode with paging active.
/// - Must be called exactly once.
pub unsafe fn init_kernel_page_table() {
    if PAGING_INITIALIZED.load(core::sync::atomic::Ordering::Relaxed) {
        log_info("PAGING", 730, "already initialized; skipping");
        return;
    }

    let mgr = PageTableManager::from_cr3();

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
        log_info("PAGING", 731, "cleared CR0.WP");
    }

    *KERNEL_PT.lock() = Some(mgr);
    PAGING_INITIALIZED.store(true, core::sync::atomic::Ordering::Release);
}

/// Returns true if the kernel page table has been initialized.
pub fn is_paging_initialized() -> bool {
    PAGING_INITIALIZED.load(core::sync::atomic::Ordering::Acquire)
}

/// Get the kernel PML4 physical address (for cloning into user page tables).
///
/// # Safety
/// Must be called after init_kernel_page_table().
pub unsafe fn kernel_pml4_phys() -> u64 {
    KERNEL_PT
        .lock()
        .as_ref()
        .expect("kernel page table not initialized")
        .pml4_phys
}

// CONVENIENCE WRAPPERS
// each acquires the KERNEL_PT lock for the duration of the operation

/// Map a single 4 KiB page in the kernel page table.
///
/// # Safety
/// See [`PageTableManager::map_4k`].
pub unsafe fn kmap_4k(virt: u64, phys: u64, flags: PageFlags) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .map_4k(virt, phys, flags)
}

/// Map a 2 MiB huge page in the kernel page table.
///
/// # Safety
/// See [`PageTableManager::map_2m`].
pub unsafe fn kmap_2m(virt: u64, phys: u64, flags: PageFlags) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .map_2m(virt, phys, flags)
}

/// Unmap a 4 KiB page from the kernel page table.
///
/// # Safety
/// See [`PageTableManager::unmap_4k`].
pub unsafe fn kunmap_4k(virt: u64) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .unmap_4k(virt)
}

/// Translate a virtual address to physical using the kernel page table.
///
/// Returns `None` if the address is not mapped.
///
/// # Safety
/// See [`PageTableManager::translate`].
pub unsafe fn kvirt_to_phys(virt: u64) -> Option<u64> {
    KERNEL_PT
        .lock()
        .as_ref()
        .expect("kernel page table not initialized")
        .translate(virt)
}

/// Split any huge pages along the path to `virt` in the kernel page table
/// so that a subsequent `kmap_4k()` call will succeed.
///
/// # Safety
/// See [`PageTableManager::ensure_4k_mappable`].
pub unsafe fn kensure_4k(virt: u64) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .ensure_4k_mappable(virt)
}

/// Identity-map a physical MMIO region with UC flags in the kernel page table.
///
/// Handles all cases: existing huge pages (sets UC bits), existing 4K pages
/// (sets UC bits), and unmapped regions (creates new identity-mapped entries).
///
/// # Safety
/// See [`PageTableManager::map_mmio`].
pub unsafe fn kmap_mmio(phys: u64, size: u64) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .map_mmio(phys, size)
}

/// Mark the leaf page table entry covering `virt` as uncacheable (UC).
///
/// Works with 1 GiB, 2 MiB, or 4 KiB pages.  Ideal for PCI MMIO BAR
/// regions that UEFI identity-mapped with Write-Back caching.
///
/// # Safety
/// See [`PageTableManager::mark_uncacheable`].
pub unsafe fn kmark_uncacheable(virt: u64) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .mark_uncacheable(virt)
}

// PAGE TABLE RESERVATION

/// Maximum number of page-table pages we expect to encounter.
///
/// UEFI/OVMF identity-maps up to ~12 GB of RAM.  With 2 MiB huge pages the
/// page table tree is shallow:
///   - 1 PML4    (always)
///   - up to ~6  PDPT pages (one per 512 GiB)
///   - up to ~24 PD pages   (one per 1 GiB)
///   - PT pages only if 4 KiB region exists (firmware code, MMIO, etc.)
///
/// On some firmware builds, low memory and MMIO windows are mapped with many
/// 4 KiB leaves, which can produce far more than a few hundred PT pages.
/// Keep this comfortably large so reclaim never misses live paging structures.
const MAX_PT_PAGES: usize = 8192;

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
    let mut truncated = false;

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
            if !seen {
                if count < MAX_PT_PAGES {
                    pages[count] = p;
                    count += 1;
                } else {
                    truncated = true;
                }
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

    if truncated {
        log_warn("PAGING", 733, "page-table page collection truncated");
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

    let (pt_pages, pt_count) = collect_page_table_pages();

    let mut registry = global_registry_mut();
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
            }
        }
    }

    log_ok("PAGING", 732, "reserved live page-table pages");

    reserved
}
