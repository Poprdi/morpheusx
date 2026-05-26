//! 4-level x86-64 page tables. Identity-mapped (VA=PA) post-UEFI.
//! Per-process PML4 clones for user isolation.

pub mod entry;
pub mod table;

pub use entry::{PageFlags, PageTable, PageTableEntry};
pub use table::{MappedPageSize, PageTableManager, VirtAddr};

use crate::serial::{log_info, log_ok, log_warn};
use crate::sync::SpinLock;

static KERNEL_PT: SpinLock<Option<PageTableManager>> = SpinLock::new(None);
static PAGING_INITIALIZED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Adopt the active CR3. Phase 1 of hwinit, after MemoryRegistry.
///
/// # Safety
/// Long mode + paging active. Call exactly once.
pub unsafe fn init_kernel_page_table() {
    if PAGING_INITIALIZED.load(core::sync::atomic::Ordering::Relaxed) {
        log_info("PAGING", 730, "already initialized; skipping");
        return;
    }

    let mgr = PageTableManager::from_cr3();

    // UEFI sets CR0.WP=1 + marks PT pages R/O — even ring-0 writes fault.
    let cr0: u64;
    core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
    if cr0 & (1u64 << 16) != 0 {
        core::arch::asm!(
            "mov cr0, {}",
            in(reg) cr0 & !(1u64 << 16),
            options(nomem, nostack),
        );
        log_info("PAGING", 731, "cleared CR0.WP");

        // Full TLB flush; some HW caches R/W=0 under WP=1 and faults later.
        let cr3_val: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3_val, options(nomem, nostack));
        core::arch::asm!("mov cr3, {}", in(reg) cr3_val, options(nostack));
    }

    *KERNEL_PT.lock() = Some(mgr);
    PAGING_INITIALIZED.store(true, core::sync::atomic::Ordering::Release);
}

pub fn is_paging_initialized() -> bool {
    PAGING_INITIALIZED.load(core::sync::atomic::Ordering::Acquire)
}

/// # Safety
/// `init_kernel_page_table` must have run.
pub unsafe fn kernel_pml4_phys() -> u64 {
    KERNEL_PT
        .lock()
        .as_ref()
        .expect("kernel page table not initialized")
        .pml4_phys
}

// Convenience wrappers — each takes the KERNEL_PT lock per call.

/// # Safety: see [`PageTableManager::map_4k`].
pub unsafe fn kmap_4k(virt: u64, phys: u64, flags: PageFlags) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .map_4k(virt, phys, flags)
}

/// # Safety: see [`PageTableManager::map_2m`].
pub unsafe fn kmap_2m(virt: u64, phys: u64, flags: PageFlags) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .map_2m(virt, phys, flags)
}

/// # Safety: see [`PageTableManager::unmap_4k`].
pub unsafe fn kunmap_4k(virt: u64) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .unmap_4k(virt)
}

/// # Safety: see [`PageTableManager::translate`].
pub unsafe fn kvirt_to_phys(virt: u64) -> Option<u64> {
    KERNEL_PT
        .lock()
        .as_ref()
        .expect("kernel page table not initialized")
        .translate(virt)
}

/// Splits huge entries on the walk so a later `kmap_4k` succeeds.
///
/// # Safety: see [`PageTableManager::ensure_4k_mappable`].
pub unsafe fn kensure_4k(virt: u64) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .ensure_4k_mappable(virt)
}

/// Identity-map MMIO as UC. Works on huge/4 KiB/unmapped regions.
///
/// # Safety: see [`PageTableManager::map_mmio`].
pub unsafe fn kmap_mmio(phys: u64, size: u64) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .map_mmio(phys, size)
}

/// UC on the covering leaf. Fixes UEFI's WB caching on PCI BAR identity maps.
///
/// # Safety: see [`PageTableManager::mark_uncacheable`].
pub unsafe fn kmark_uncacheable(virt: u64) -> Result<(), &'static str> {
    KERNEL_PT
        .lock()
        .as_mut()
        .expect("kernel page table not initialized")
        .mark_uncacheable(virt)
}

/// UEFI's 2 MiB tree fits in dozens; some firmware uses 4 KiB leaves over
/// MMIO and balloons into thousands. Keep this large.
const MAX_PT_PAGES: usize = 8192;

/// Walks CR3 directly so it can run before `init_kernel_page_table`.
///
/// # Safety
/// Long mode + paging active, identity-mapped.
pub unsafe fn collect_page_table_pages() -> ([u64; MAX_PT_PAGES], usize) {
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
    let pml4_phys = cr3 & !0xFFFu64;

    let mut pages = [0u64; MAX_PT_PAGES];
    let mut count = 0usize;
    let mut truncated = false;

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

    add_page!(pml4_phys);

    let pml4 = pml4_phys as *const u64;
    for i in 0..512usize {
        let e1 = *pml4.add(i);
        if e1 & 1 == 0 {
            continue;
        }
        let pdpt_phys = e1 & 0x000F_FFFF_FFFF_F000;
        add_page!(pdpt_phys);

        let pdpt = pdpt_phys as *const u64;
        for j in 0..512usize {
            let e2 = *pdpt.add(j);
            if e2 & 1 == 0 {
                continue;
            }
            if e2 & (1 << 7) != 0 {
                // 1 GiB huge — no PD.
                continue;
            }
            let pd_phys = e2 & 0x000F_FFFF_FFFF_F000;
            add_page!(pd_phys);

            let pd = pd_phys as *const u64;
            for k in 0..512usize {
                let e3 = *pd.add(k);
                if e3 & 1 == 0 {
                    continue;
                }
                if e3 & (1 << 7) != 0 {
                    // 2 MiB huge — no PT.
                    continue;
                }
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

/// Pin every live PT page. MUST run after registry init but before any
/// free-list alloc, or we risk handing out a live PT page.
///
/// # Safety
/// Identity-mapped long mode; registry initialized.
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
            },
            Err(_) => {
                // Already non-free (e.g. RuntimeServices); allocator can't hand it out.
            },
        }
    }

    log_ok("PAGING", 732, "reserved live page-table pages");

    reserved
}
