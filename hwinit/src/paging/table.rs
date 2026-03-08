//! Page Table Manager — x86-64 4-level paging
//!
//! Manages the live page tables after ExitBootServices.  UEFI leaves the CPU
//! in 64-bit long mode with an identity-mapped address space (phys == virt for
//! all RAM up to the firmware limit).  We read CR3 to find that PML4, then
//! extend it with new mappings as needed.
//!
//! ## Identity mapping assumption
//!
//! As long as paging is identity-mapped, a *physical address* returned by
//! `MemoryRegistry::allocate_pages()` is also a valid *pointer* we can
//! dereference directly.  This assumption breaks once we switch to per-process
//! address spaces (Phase 3+); at that point new page tables will be mapped into
//! the kernel virtual address space via explicit `map_page()` calls.
//!
//! ## Safety
//!
//! All methods that touch page table memory or the CR3 register are inherently
//! unsafe.  Callers must ensure:
//! - No other code modifies the same page table concurrently.
//! - Addresses are canonical (bits 48..63 are sign-extensions of bit 47).
//! - Mappings do not alias kernel code/data in destructive ways.

use super::entry::{PageFlags, PageTable, PageTableEntry};
use crate::memory::{
    global_registry_mut, is_registry_initialized, AllocateType, MemoryType, PAGE_SIZE,
};

// VIRTUAL ADDRESS DECOMPOSITION

/// Decompose a 64-bit virtual address into 4-level page table indices + offset.
///
/// ```text
///  63      48 47     39 38     30 29     21 20     12 11       0
/// sign    │  PML4   │  PDPT   │   PD    │   PT    │  offset
/// extend  │ [8:0]   │ [8:0]   │  [8:0]  │  [8:0]  │  [11:0]
/// ```
#[derive(Debug, Clone, Copy)]
pub struct VirtAddr {
    pub pml4_idx: usize, // bits 47..39
    pub pdpt_idx: usize, // bits 38..30
    pub pd_idx: usize,   // bits 29..21
    pub pt_idx: usize,   // bits 20..12
    pub page_off: usize, // bits 11..0
}

impl VirtAddr {
    pub const fn from_u64(virt: u64) -> Self {
        Self {
            pml4_idx: ((virt >> 39) & 0x1FF) as usize,
            pdpt_idx: ((virt >> 30) & 0x1FF) as usize,
            pd_idx: ((virt >> 21) & 0x1FF) as usize,
            pt_idx: ((virt >> 12) & 0x1FF) as usize,
            page_off: (virt & 0xFFF) as usize,
        }
    }
}

// PAGE SIZE VARIANTS

/// The size of pages supported by the mapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappedPageSize {
    /// Standard 4 KiB page (maps a PT entry).
    Size4K,
    /// 2 MiB huge page (maps a PD entry with the PS bit set).
    Size2M,
}

// PAGE TABLE MANAGER

/// Provides high-level operations over the x86-64 4-level page table tree.
///
/// Created by `PageTableManager::from_cr3()` which reads the current CR3,
/// or `PageTableManager::new_empty()` which allocates a fresh PML4.
pub struct PageTableManager {
    /// Physical (= virtual, identity-mapped) address of the PML4 table.
    pub pml4_phys: u64,
}

impl PageTableManager {
    // construction

    /// Initialise from the currently active CR3 register.
    ///
    /// This is the normal entry point post-EBS: UEFI has already set up a
    /// working identity map; we adopt its PML4 and extend it.
    ///
    /// # Safety
    /// Must be called in long mode with paging active.
    pub unsafe fn from_cr3() -> Self {
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
        // CR3[51:12] = PML4 physical base; lower bits are flags (PCID etc.)
        let pml4_phys = cr3 & 0x000F_FFFF_FFFF_F000;
        Self { pml4_phys }
    }

    /// Allocate a fresh, zeroed PML4 from the MemoryRegistry.
    ///
    /// # Safety
    /// MemoryRegistry must be initialized.  Call `load()` to activate this
    /// table (writes it into CR3).
    pub unsafe fn new_empty() -> Result<Self, &'static str> {
        if !is_registry_initialized() {
            return Err("MemoryRegistry not initialized");
        }
        let phys = alloc_table()?;
        Ok(Self { pml4_phys: phys })
    }

    // cr3 operations

    /// Load this page table as the active page table (writes CR3).
    ///
    /// # Safety
    /// The PML4 must correctly map all memory the CPU might need to access
    /// immediately after the write (stack, code, IDT, GDT, etc.).
    pub unsafe fn load(&self) {
        core::arch::asm!(
            "mov cr3, {}",
            in(reg) self.pml4_phys,
            options(nostack, nomem)
        );
    }

    /// Flush the TLB entry for a single virtual address.
    #[inline]
    pub unsafe fn flush_tlb_page(virt: u64) {
        core::arch::asm!("invlpg [{addr}]", addr = in(reg) virt, options(nostack));
    }

    // mapping

    /// Map `virt` → `phys` using 4 KiB pages with the given flags.
    ///
    /// Intermediate page tables (PDPT, PD, PT) are allocated on demand from
    /// MemoryRegistry with `KERNEL_RW` flags so the kernel can walk them.
    ///
    /// - `virt` and `phys` must be 4 KiB-aligned.
    /// - Overwrites any existing mapping (does NOT check for conflicts).
    ///
    /// # Safety
    /// See module-level safety note.
    pub unsafe fn map_4k(
        &mut self,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), &'static str> {
        let va = VirtAddr::from_u64(virt);

        // PML4 → PDPT
        let pml4 = self.pml4_phys as *mut PageTable;
        let pdpt_phys = ensure_table((*pml4).entry_mut(va.pml4_idx))?;

        // PDPT → PD
        let pdpt = pdpt_phys as *mut PageTable;
        let pd_phys = ensure_table((*pdpt).entry_mut(va.pdpt_idx))?;

        // PD → PT
        let pd = pd_phys as *mut PageTable;
        let e = (*pd).entry_mut(va.pd_idx);
        if e.is_present() && e.is_huge() {
            return Err("map_4k: target PD entry is a 2 MiB huge page");
        }
        let pt_phys = ensure_table(e)?;

        // write PT entry
        let pt = pt_phys as *mut PageTable;
        *(*pt).entry_mut(va.pt_idx) = PageTableEntry::new(phys, flags.with(PageFlags::PRESENT));

        Self::flush_tlb_page(virt);
        Ok(())
    }

    /// Map `virt` → `phys` using a 2 MiB huge page.
    ///
    /// - Both addresses must be 2 MiB-aligned.
    ///
    /// # Safety
    /// See module-level safety note.
    pub unsafe fn map_2m(
        &mut self,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), &'static str> {
        if virt & 0x1F_FFFF != 0 || phys & 0x1F_FFFF != 0 {
            return Err("map_2m: addresses must be 2 MiB-aligned");
        }
        let va = VirtAddr::from_u64(virt);

        let pml4 = self.pml4_phys as *mut PageTable;
        let pdpt_phys = ensure_table((*pml4).entry_mut(va.pml4_idx))?;
        let pdpt = pdpt_phys as *mut PageTable;
        let pd_phys = ensure_table((*pdpt).entry_mut(va.pdpt_idx))?;

        let pd = pd_phys as *mut PageTable;
        let e = (*pd).entry_mut(va.pd_idx);
        *e = PageTableEntry::new(
            phys,
            flags.with(PageFlags::PRESENT).with(PageFlags::HUGE_PAGE),
        );

        Self::flush_tlb_page(virt);
        Ok(())
    }

    // unmapping

    /// Unmap the 4 KiB page at `virt`.
    ///
    /// Does nothing (returns `Ok`) if the page is not mapped.  Does NOT free
    /// the intermediate page tables even if they become empty.
    ///
    /// # Safety
    /// See module-level safety note.
    pub unsafe fn unmap_4k(&mut self, virt: u64) -> Result<(), &'static str> {
        let va = VirtAddr::from_u64(virt);

        let pml4 = self.pml4_phys as *mut PageTable;
        let pml4_e = (*pml4).entry(va.pml4_idx);
        if !pml4_e.is_present() {
            return Ok(());
        }

        let pdpt = pml4_e.phys_addr() as *mut PageTable;
        let pdpt_e = (*pdpt).entry(va.pdpt_idx);
        if !pdpt_e.is_present() {
            return Ok(());
        }

        let pd = pdpt_e.phys_addr() as *mut PageTable;
        let pd_e = (*pd).entry(va.pd_idx);
        if !pd_e.is_present() {
            return Ok(());
        }
        if pd_e.is_huge() {
            return Err("unmap_4k: PD entry is a 2 MiB huge page; use unmap_2m");
        }

        let pt = pd_e.phys_addr() as *mut PageTable;
        (*pt).entry_mut(va.pt_idx).clear();

        Self::flush_tlb_page(virt);
        Ok(())
    }

    /// Unmap the 2 MiB huge page at `virt`.
    ///
    /// # Safety
    /// See module-level safety note.
    pub unsafe fn unmap_2m(&mut self, virt: u64) -> Result<(), &'static str> {
        let va = VirtAddr::from_u64(virt);

        let pml4 = self.pml4_phys as *mut PageTable;
        let pml4_e = (*pml4).entry(va.pml4_idx);
        if !pml4_e.is_present() {
            return Ok(());
        }

        let pdpt = pml4_e.phys_addr() as *mut PageTable;
        let pdpt_e = (*pdpt).entry(va.pdpt_idx);
        if !pdpt_e.is_present() {
            return Ok(());
        }

        let pd = pdpt_e.phys_addr() as *mut PageTable;
        let e = (*pd).entry_mut(va.pd_idx);
        e.clear();

        Self::flush_tlb_page(virt);
        Ok(())
    }

    // translation

    /// Walk the page tables to translate `virt` → physical address.
    ///
    /// Returns `None` if any level is not present.
    ///
    /// # Safety
    /// See module-level safety note.
    pub unsafe fn translate(&self, virt: u64) -> Option<u64> {
        let va = VirtAddr::from_u64(virt);

        let pml4 = self.pml4_phys as *const PageTable;
        let pml4_e = (*pml4).entry(va.pml4_idx);
        if !pml4_e.is_present() {
            return None;
        }

        let pdpt = pml4_e.phys_addr() as *const PageTable;
        let pdpt_e = (*pdpt).entry(va.pdpt_idx);
        if !pdpt_e.is_present() {
            return None;
        }
        // 1 GiB huge page
        if pdpt_e.is_huge() {
            let base = pdpt_e.phys_addr();
            let off = virt & 0x3FFF_FFFF;
            return Some(base | off);
        }

        let pd = pdpt_e.phys_addr() as *const PageTable;
        let pd_e = (*pd).entry(va.pd_idx);
        if !pd_e.is_present() {
            return None;
        }
        // 2 MiB huge page
        if pd_e.is_huge() {
            let base = pd_e.phys_addr();
            let off = virt & 0x1F_FFFF;
            return Some(base | off);
        }

        let pt = pd_e.phys_addr() as *const PageTable;
        let pt_e = (*pt).entry(va.pt_idx);
        if !pt_e.is_present() {
            return None;
        }

        Some(pt_e.phys_addr() | va.page_off as u64)
    }

    // identity-map a contiguous physical range

    /// Identity-map `[phys_base, phys_base + size)` using 2 MiB huge pages
    /// where aligned, falling back to 4 KiB pages for unaligned edges.
    ///
    /// Skips ranges that are already mapped.
    ///
    /// # Safety
    /// See module-level safety note.
    pub unsafe fn identity_map_range(
        &mut self,
        phys_base: u64,
        size: u64,
        flags: PageFlags,
    ) -> Result<(), &'static str> {
        let two_mb: u64 = 2 * 1024 * 1024;
        let mut cur = phys_base;
        let end = phys_base + size;

        while cur < end {
            let remaining = end - cur;
            if cur & (two_mb - 1) == 0 && remaining >= two_mb {
                self.map_2m(cur, cur, flags)?;
                cur += two_mb;
            } else {
                self.map_4k(cur, cur, flags)?;
                cur += PAGE_SIZE;
            }
        }
        Ok(())
    }

    // mmio mapping

    /// Identity-map a physical MMIO region with Uncacheable (UC) flags.
    ///
    /// For each 4 KiB page in `[phys, phys + size)` (page-aligned):
    ///   - If part of an existing 1 GiB or 2 MiB huge page → set UC bits
    ///     on the huge-page leaf entry and advance to its next boundary.
    ///   - If already mapped as a 4 KiB page → set UC bits on that entry.
    ///   - If NOT mapped at all → create a new identity-mapped 4 KiB entry
    ///     with UC + PRESENT + WRITABLE + NX.  Intermediate tables are
    ///     allocated from `MemoryRegistry` on demand.
    ///
    /// A full TLB flush (CR3 reload) is performed once at the end.
    ///
    /// # Safety
    /// See module-level safety note.  MemoryRegistry must be initialized
    /// so that intermediate page tables can be allocated when needed.
    pub unsafe fn map_mmio(&mut self, phys: u64, size: u64) -> Result<(), &'static str> {
        // Disable interrupts for the entire page table modification.
        // The PIT timer ISR (100 Hz) does context save/restore through the
        // same page tables we're editing — interleaving is unsafe.
        let rflags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem, nostack));
        core::arch::asm!("cli", options(nomem, nostack));

        let result = self.map_mmio_inner(phys, size);

        // Restore previous interrupt state.
        if rflags & 0x200 != 0 {
            core::arch::asm!("sti", options(nomem, nostack));
        }
        result
    }

    /// Inner implementation of `map_mmio` — caller must have disabled
    /// interrupts before calling.
    unsafe fn map_mmio_inner(&mut self, phys: u64, size: u64) -> Result<(), &'static str> {
        let uc_bits = PageFlags::CACHE_DISABLE.0 | PageFlags::WRITE_THROUGH.0;
        let new_flags = PageFlags::PRESENT
            .with(PageFlags::WRITABLE)
            .with(PageFlags::CACHE_DISABLE)
            .with(PageFlags::WRITE_THROUGH)
            .with(PageFlags::NO_EXECUTE);

        let start = phys & !0xFFF;
        let end = (phys + size + 0xFFF) & !0xFFF;
        let mut cur = start;

        while cur < end {
            let va = VirtAddr::from_u64(cur);
            let pml4 = self.pml4_phys as *mut PageTable;

            // pml4
            let pml4_e = (*pml4).entry_mut(va.pml4_idx);
            if !pml4_e.is_present() {
                let child = alloc_table()?;
                *pml4_e = PageTableEntry::new(child, PageFlags::PRESENT.with(PageFlags::WRITABLE));
            }

            // pdpt
            let pdpt = pml4_e.phys_addr() as *mut PageTable;
            let pdpt_e = (*pdpt).entry_mut(va.pdpt_idx);

            if !pdpt_e.is_present() {
                let child = alloc_table()?;
                *pdpt_e = PageTableEntry::new(child, PageFlags::PRESENT.with(PageFlags::WRITABLE));
            }
            if pdpt_e.is_huge() {
                pdpt_e.set_raw(pdpt_e.raw() | uc_bits);
                let next_1g = (cur & !(0x4000_0000 - 1)) + 0x4000_0000;
                cur = next_1g;
                continue;
            }

            // pd
            let pd = pdpt_e.phys_addr() as *mut PageTable;
            let pd_e = (*pd).entry_mut(va.pd_idx);

            if !pd_e.is_present() {
                let child = alloc_table()?;
                *pd_e = PageTableEntry::new(child, PageFlags::PRESENT.with(PageFlags::WRITABLE));
            }
            if pd_e.is_huge() {
                pd_e.set_raw(pd_e.raw() | uc_bits);
                let next_2m = (cur & !(0x20_0000 - 1)) + 0x20_0000;
                cur = next_2m;
                continue;
            }

            // pt (4 kib leaf)
            let pt = pd_e.phys_addr() as *mut PageTable;
            let pt_e = (*pt).entry_mut(va.pt_idx);

            if pt_e.is_present() {
                pt_e.set_raw(pt_e.raw() | uc_bits);
            } else {
                *pt_e = PageTableEntry::new(cur, new_flags);
            }
            cur += PAGE_SIZE;
        }

        // WBINVD: write-back and invalidate ALL caches.  Critical when
        // changing a region from WB → UC — stale WB cache lines from
        // UEFI's PCI enumeration can shadow the real device registers.
        core::arch::asm!("wbinvd", options(nomem, nostack));
        self.flush_tlb_all();
        Ok(())
    }

    /// Mark the page table mapping covering `virt` as uncacheable (UC).
    ///
    /// Convenience wrapper: calls `map_mmio(virt, PAGE_SIZE)` to handle
    /// both mapped and unmapped pages.
    ///
    /// # Safety
    /// See module-level safety note.
    pub unsafe fn mark_uncacheable(&mut self, virt: u64) -> Result<(), &'static str> {
        self.map_mmio(virt, PAGE_SIZE)
    }

    /// Flush the entire TLB by reloading CR3.
    #[inline]
    pub unsafe fn flush_tlb_all(&self) {
        core::arch::asm!(
            "mov {tmp}, cr3",
            "mov cr3, {tmp}",
            tmp = out(reg) _,
            options(nostack, nomem)
        );
    }

    // huge-page splitting

    /// Split any huge pages along the path to `virt` so that `map_4k()` can
    /// proceed without hitting a "tried to walk through a huge-page entry"
    /// error.
    ///
    /// If the path contains a 1 GiB huge page at the PDPT level, it is split
    /// into 512 × 2 MiB pages.  If it contains a 2 MiB huge page at the PD
    /// level, it is split into 512 × 4 KiB pages.  Both levels are handled
    /// in a single call.
    ///
    /// After `ensure_4k_mappable(virt)`, `map_4k(virt, phys, flags)` is
    /// guaranteed not to fail due to pre-existing huge pages.
    ///
    /// # Safety
    /// See module-level safety note.  Allocates new page table pages from
    /// the MemoryRegistry.
    pub unsafe fn ensure_4k_mappable(&mut self, virt: u64) -> Result<(), &'static str> {
        let va = VirtAddr::from_u64(virt);

        let pml4 = self.pml4_phys as *mut PageTable;

        // level 1: pml4[idx] → pdpt
        let pml4_e = (*pml4).entry_mut(va.pml4_idx);
        if !pml4_e.is_present() {
            // Not mapped at all — map_4k will allocate as needed.
            return Ok(());
        }
        let pdpt_phys = pml4_e.phys_addr();

        // level 2: pdpt[idx] → pd
        let pdpt = pdpt_phys as *mut PageTable;
        let pdpt_e = (*pdpt).entry_mut(va.pdpt_idx);
        if !pdpt_e.is_present() {
            return Ok(());
        }
        if pdpt_e.is_huge() {
            // 1 GiB huge page — split into 512 × 2 MiB pages.
            let huge_base = pdpt_e.phys_addr();
            let raw = pdpt_e.raw();
            // Propagate: P, W, U, PWT, PCD, A, G, XD (NOT PS — we set it per-entry)
            let leaf_flags = PageFlags(
                raw & (PageFlags::PRESENT.0
                    | PageFlags::WRITABLE.0
                    | PageFlags::USER.0
                    | PageFlags::WRITE_THROUGH.0
                    | PageFlags::CACHE_DISABLE.0
                    | PageFlags::ACCESSED.0
                    | PageFlags::GLOBAL.0
                    | PageFlags::NO_EXECUTE.0),
            );

            let new_pd_phys = alloc_table()?;
            let new_pd = new_pd_phys as *mut PageTable;
            let two_mb: u64 = 2 * 1024 * 1024;
            for i in 0..512usize {
                let page_phys = huge_base + (i as u64) * two_mb;
                *(*new_pd).entry_mut(i) =
                    PageTableEntry::new(page_phys, leaf_flags.with(PageFlags::HUGE_PAGE));
            }

            // Replace PDPT entry: directory (not huge) pointing to the new PD.
            let dir_flags = PageFlags::PRESENT.with(PageFlags::WRITABLE);
            let dir_flags = if leaf_flags.contains(PageFlags::USER) {
                dir_flags.with(PageFlags::USER)
            } else {
                dir_flags
            };
            *pdpt_e = PageTableEntry::new(new_pd_phys, dir_flags);

            Self::flush_tlb_page(virt);
        }

        // Re-read PDPT entry (may have been replaced above).
        let pd_phys = (*pdpt).entry(va.pdpt_idx).phys_addr();

        // level 3: pd[idx] → pt
        let pd = pd_phys as *mut PageTable;
        let pd_e = (*pd).entry_mut(va.pd_idx);
        if !pd_e.is_present() {
            return Ok(());
        }
        if pd_e.is_huge() {
            // 2 MiB huge page — split into 512 × 4 KiB pages.
            let huge_base = pd_e.phys_addr();
            let raw = pd_e.raw();
            let leaf_flags = PageFlags(
                raw & (PageFlags::PRESENT.0
                    | PageFlags::WRITABLE.0
                    | PageFlags::USER.0
                    | PageFlags::WRITE_THROUGH.0
                    | PageFlags::CACHE_DISABLE.0
                    | PageFlags::ACCESSED.0
                    | PageFlags::GLOBAL.0
                    | PageFlags::NO_EXECUTE.0),
            );

            let new_pt_phys = alloc_table()?;
            let new_pt = new_pt_phys as *mut PageTable;
            for i in 0..512usize {
                let page_phys = huge_base + (i as u64) * PAGE_SIZE;
                *(*new_pt).entry_mut(i) = PageTableEntry::new(page_phys, leaf_flags);
            }

            // Replace PD entry: directory (not huge) pointing to the new PT.
            let dir_flags = PageFlags::PRESENT.with(PageFlags::WRITABLE);
            let dir_flags = if leaf_flags.contains(PageFlags::USER) {
                dir_flags.with(PageFlags::USER)
            } else {
                dir_flags
            };
            *pd_e = PageTableEntry::new(new_pt_phys, dir_flags);
        }

        Self::flush_tlb_page(virt);

        Ok(())
    }
}

// HELPERS

/// Allocate a single zeroed page from MemoryRegistry for use as a page table.
unsafe fn alloc_table() -> Result<u64, &'static str> {
    if !is_registry_initialized() {
        return Err("cannot allocate page table: registry not initialized");
    }
    let mut registry = global_registry_mut();
    registry
        .allocate_pages(AllocateType::AnyPages, MemoryType::AllocatedPageTable, 1)
        .inspect(|&phys| {
            // Zero the freshly allocated page.
            let p = phys as *mut PageTable;
            (*p).zero();
        })
        .map_err(|_| "page table allocation failed")
}

/// If the page table entry `e` already points to a child table, return its
/// physical address.  Otherwise allocate a new table, fill `e` with a
/// present + writable kernel pointer to it, and return its address.
unsafe fn ensure_table(e: &mut PageTableEntry) -> Result<u64, &'static str> {
    if e.is_present() {
        if e.is_huge() {
            return Err("paging: tried to walk through a huge-page entry");
        }
        return Ok(e.phys_addr());
    }

    // Allocate and fill.
    let child_phys = alloc_table()?;
    *e = PageTableEntry::new(child_phys, PageFlags::PRESENT.with(PageFlags::WRITABLE));
    Ok(child_phys)
}
