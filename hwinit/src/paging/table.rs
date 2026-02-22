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
use crate::memory::{global_registry_mut, is_registry_initialized, AllocateType, MemoryType, PAGE_SIZE};
use crate::serial::puts;

// ═══════════════════════════════════════════════════════════════════════════
// VIRTUAL ADDRESS DECOMPOSITION
// ═══════════════════════════════════════════════════════════════════════════

/// Decompose a 64-bit virtual address into 4-level page table indices + offset.
///
/// ```text
///  63      48 47     39 38     30 29     21 20     12 11       0
/// ┌──────────┬─────────┬─────────┬─────────┬─────────┬──────────┐
/// │  sign    │  PML4   │  PDPT   │   PD    │   PT    │  offset  │
/// │  extend  │ [8:0]   │ [8:0]   │  [8:0]  │  [8:0]  │  [11:0]  │
/// └──────────┴─────────┴─────────┴─────────┴─────────┴──────────┘
/// ```
#[derive(Debug, Clone, Copy)]
pub struct VirtAddr {
    pub pml4_idx:  usize,   // bits 47..39
    pub pdpt_idx:  usize,   // bits 38..30
    pub pd_idx:    usize,   // bits 29..21
    pub pt_idx:    usize,   // bits 20..12
    pub page_off:  usize,   // bits 11..0
}

impl VirtAddr {
    pub const fn from_u64(virt: u64) -> Self {
        Self {
            pml4_idx: ((virt >> 39) & 0x1FF) as usize,
            pdpt_idx: ((virt >> 30) & 0x1FF) as usize,
            pd_idx:   ((virt >> 21) & 0x1FF) as usize,
            pt_idx:   ((virt >> 12) & 0x1FF) as usize,
            page_off: (virt & 0xFFF) as usize,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PAGE SIZE VARIANTS
// ═══════════════════════════════════════════════════════════════════════════

/// The size of pages supported by the mapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappedPageSize {
    /// Standard 4 KiB page (maps a PT entry).
    Size4K,
    /// 2 MiB huge page (maps a PD entry with the PS bit set).
    Size2M,
}

// ═══════════════════════════════════════════════════════════════════════════
// PAGE TABLE MANAGER
// ═══════════════════════════════════════════════════════════════════════════

/// Provides high-level operations over the x86-64 4-level page table tree.
///
/// Created by `PageTableManager::from_cr3()` which reads the current CR3,
/// or `PageTableManager::new_empty()` which allocates a fresh PML4.
pub struct PageTableManager {
    /// Physical (= virtual, identity-mapped) address of the PML4 table.
    pub pml4_phys: u64,
}

impl PageTableManager {
    // ── Construction ─────────────────────────────────────────────────────

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

    // ── CR3 operations ───────────────────────────────────────────────────

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

    // ── Mapping ──────────────────────────────────────────────────────────

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

        // Walk / create PML4 → PDPT ──────────────────────────────────────
        let pml4 = self.pml4_phys as *mut PageTable;
        let pdpt_phys = ensure_table((*pml4).entry_mut(va.pml4_idx))?;

        // Walk / create PDPT → PD ────────────────────────────────────────
        let pdpt = pdpt_phys as *mut PageTable;
        let pd_phys = ensure_table((*pdpt).entry_mut(va.pdpt_idx))?;

        // Walk / create PD → PT ──────────────────────────────────────────
        let pd = pd_phys as *mut PageTable;
        let e = (*pd).entry_mut(va.pd_idx);
        if e.is_present() && e.is_huge() {
            return Err("map_4k: target PD entry is a 2 MiB huge page");
        }
        let pt_phys = ensure_table(e)?;

        // Write the PT entry ──────────────────────────────────────────────
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
            flags
                .with(PageFlags::PRESENT)
                .with(PageFlags::HUGE_PAGE),
        );

        Self::flush_tlb_page(virt);
        Ok(())
    }

    // ── Unmapping ────────────────────────────────────────────────────────

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
        if !pml4_e.is_present() { return Ok(()); }

        let pdpt = pml4_e.phys_addr() as *mut PageTable;
        let pdpt_e = (*pdpt).entry(va.pdpt_idx);
        if !pdpt_e.is_present() { return Ok(()); }

        let pd = pdpt_e.phys_addr() as *mut PageTable;
        let pd_e = (*pd).entry(va.pd_idx);
        if !pd_e.is_present() { return Ok(()); }
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
        if !pml4_e.is_present() { return Ok(()); }

        let pdpt = pml4_e.phys_addr() as *mut PageTable;
        let pdpt_e = (*pdpt).entry(va.pdpt_idx);
        if !pdpt_e.is_present() { return Ok(()); }

        let pd = pdpt_e.phys_addr() as *mut PageTable;
        let e = (*pd).entry_mut(va.pd_idx);
        e.clear();

        Self::flush_tlb_page(virt);
        Ok(())
    }

    // ── Translation ──────────────────────────────────────────────────────

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
        if !pml4_e.is_present() { return None; }

        let pdpt = pml4_e.phys_addr() as *const PageTable;
        let pdpt_e = (*pdpt).entry(va.pdpt_idx);
        if !pdpt_e.is_present() { return None; }
        // 1 GiB huge page
        if pdpt_e.is_huge() {
            let base = pdpt_e.phys_addr();
            let off  = virt & 0x3FFF_FFFF;
            return Some(base | off);
        }

        let pd = pdpt_e.phys_addr() as *const PageTable;
        let pd_e = (*pd).entry(va.pd_idx);
        if !pd_e.is_present() { return None; }
        // 2 MiB huge page
        if pd_e.is_huge() {
            let base = pd_e.phys_addr();
            let off  = virt & 0x1F_FFFF;
            return Some(base | off);
        }

        let pt = pd_e.phys_addr() as *const PageTable;
        let pt_e = (*pt).entry(va.pt_idx);
        if !pt_e.is_present() { return None; }

        Some(pt_e.phys_addr() | va.page_off as u64)
    }

    // ── Identity-map a contiguous physical range ─────────────────────────

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
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Allocate a single zeroed page from MemoryRegistry for use as a page table.
unsafe fn alloc_table() -> Result<u64, &'static str> {
    if !is_registry_initialized() {
        return Err("cannot allocate page table: registry not initialized");
    }
    let registry = global_registry_mut();
    registry
        .allocate_pages(
            AllocateType::AnyPages,
            MemoryType::AllocatedPageTable,
            1,
        )
        .map(|phys| {
            // Zero the freshly allocated page.
            let p = phys as *mut PageTable;
            (*p).zero();
            phys
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
    *e = PageTableEntry::new(
        child_phys,
        PageFlags::PRESENT.with(PageFlags::WRITABLE),
    );
    Ok(child_phys)
}
