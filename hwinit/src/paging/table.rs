//! x86-64 4-level page table manager. Identity-mapped, so phys addrs returned
//! from MemoryRegistry are also valid pointers. AMD64 Vol 2 §5.3.

use super::entry::{PageFlags, PageTable, PageTableEntry};
use crate::memory::{
    global_registry_mut, is_registry_initialized, AllocateType, MemoryType, PAGE_SIZE,
};

/// 4-level decomposition: PML4 [47:39], PDPT [38:30], PD [29:21], PT [20:12], off [11:0].
#[derive(Debug, Clone, Copy)]
pub struct VirtAddr {
    pub pml4_idx: usize,
    pub pdpt_idx: usize,
    pub pd_idx: usize,
    pub pt_idx: usize,
    pub page_off: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappedPageSize {
    Size4K,
    /// 2 MiB huge page (PD entry with PS=1).
    Size2M,
}

pub struct PageTableManager {
    /// PML4 physical address (= virtual, identity-mapped).
    pub pml4_phys: u64,
}

impl PageTableManager {
    /// Adopt the active CR3. AMD64 Vol 2 §5.3 — CR3[51:12] = PML4 base.
    ///
    /// # Safety
    /// Long mode + paging active.
    pub unsafe fn from_cr3() -> Self {
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
        let pml4_phys = cr3 & 0x000F_FFFF_FFFF_F000;
        Self { pml4_phys }
    }

    /// # Safety
    /// MemoryRegistry must be initialized. Call `load()` to activate.
    pub unsafe fn new_empty() -> Result<Self, &'static str> {
        if !is_registry_initialized() {
            return Err("MemoryRegistry not initialized");
        }
        let phys = alloc_table()?;
        Ok(Self { pml4_phys: phys })
    }

    /// Activate by writing CR3.
    ///
    /// # Safety
    /// PML4 must cover everything the CPU touches post-write (stack, code, IDT, GDT).
    pub unsafe fn load(&self) {
        core::arch::asm!(
            "mov cr3, {}",
            in(reg) self.pml4_phys,
            options(nostack, nomem)
        );
    }

    #[inline]
    pub unsafe fn flush_tlb_page(virt: u64) {
        core::arch::asm!("invlpg [{addr}]", addr = in(reg) virt, options(nostack));
    }

    /// Map `virt` → `phys`, 4 KiB-aligned. Overwrites silently; allocates
    /// intermediate PDPT/PD/PT pages with KERNEL_RW as needed.
    ///
    /// # Safety
    /// No concurrent modification, canonical address, no destructive aliasing.
    pub unsafe fn map_4k(
        &mut self,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), &'static str> {
        let va = VirtAddr::from_u64(virt);

        let pml4 = self.pml4_phys as *mut PageTable;
        let pdpt_phys = ensure_table((*pml4).entry_mut(va.pml4_idx))?;

        let pdpt = pdpt_phys as *mut PageTable;
        let pd_phys = ensure_table((*pdpt).entry_mut(va.pdpt_idx))?;

        let pd = pd_phys as *mut PageTable;
        let e = (*pd).entry_mut(va.pd_idx);
        if e.is_present() && e.is_huge() {
            return Err("map_4k: target PD entry is a 2 MiB huge page");
        }
        let pt_phys = ensure_table(e)?;

        let pt = pt_phys as *mut PageTable;
        *(*pt).entry_mut(va.pt_idx) = PageTableEntry::new(phys, flags.with(PageFlags::PRESENT));

        Self::flush_tlb_page(virt);
        Ok(())
    }

    /// `virt` and `phys` must be 2 MiB-aligned.
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

    /// No-op on unmapped pages. Intermediate tables are not freed.
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

    /// Walk to physical; `None` if any level is not present.
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

    /// Identity-map `[phys_base, +size)` with 2 MiB pages where aligned,
    /// 4 KiB on the edges.
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

    /// Identity-map MMIO with UC (PCD|PWT). Walks each 4 KiB page in range:
    /// existing huge → set UC and skip to next huge boundary; existing 4 KiB
    /// → set UC; absent → create UC+P+W+NX entry. Followed by WBINVD + full
    /// TLB flush.
    pub unsafe fn map_mmio(&mut self, phys: u64, size: u64) -> Result<(), &'static str> {
        // PIT ISR walks these same tables — block interrupts for the edit.
        let rflags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem, nostack));
        core::arch::asm!("cli", options(nomem, nostack));

        let result = self.map_mmio_inner(phys, size);

        if rflags & 0x200 != 0 {
            core::arch::asm!("sti", options(nomem, nostack));
        }
        result
    }

    /// Caller must have disabled interrupts.
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

            let pml4_e = (*pml4).entry_mut(va.pml4_idx);
            if !pml4_e.is_present() {
                let child = alloc_table()?;
                *pml4_e = PageTableEntry::new(child, PageFlags::PRESENT.with(PageFlags::WRITABLE));
            }

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

            let pt = pd_e.phys_addr() as *mut PageTable;
            let pt_e = (*pt).entry_mut(va.pt_idx);

            if pt_e.is_present() {
                pt_e.set_raw(pt_e.raw() | uc_bits);
            } else {
                *pt_e = PageTableEntry::new(cur, new_flags);
            }
            cur += PAGE_SIZE;
        }

        // WBINVD: stale WB lines from UEFI PCI enumeration can shadow
        // device regs after the WB→UC switch. AMD64 Vol 2 §7.5.
        core::arch::asm!("wbinvd", options(nomem, nostack));
        self.flush_tlb_all();
        Ok(())
    }

    pub unsafe fn mark_uncacheable(&mut self, virt: u64) -> Result<(), &'static str> {
        self.map_mmio(virt, PAGE_SIZE)
    }

    #[inline]
    pub unsafe fn flush_tlb_all(&self) {
        core::arch::asm!(
            "mov {tmp}, cr3",
            "mov cr3, {tmp}",
            tmp = out(reg) _,
            options(nostack, nomem)
        );
    }

    /// Split 1 GiB → 2 MiB and 2 MiB → 4 KiB along the path to `virt` so a
    /// later `map_4k` won't hit a huge entry. Both levels in one call.
    pub unsafe fn ensure_4k_mappable(&mut self, virt: u64) -> Result<(), &'static str> {
        let va = VirtAddr::from_u64(virt);

        let pml4 = self.pml4_phys as *mut PageTable;

        let pml4_e = (*pml4).entry_mut(va.pml4_idx);
        if !pml4_e.is_present() {
            return Ok(());
        }
        let pdpt_phys = pml4_e.phys_addr();

        let pdpt = pdpt_phys as *mut PageTable;
        let pdpt_e = (*pdpt).entry_mut(va.pdpt_idx);
        if !pdpt_e.is_present() {
            return Ok(());
        }
        if pdpt_e.is_huge() {
            // 1 GiB → 512 × 2 MiB. Propagate P/W/U/PWT/PCD/A/G/XD, set PS per leaf.
            let huge_base = pdpt_e.phys_addr();
            let raw = pdpt_e.raw();
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

            let dir_flags = PageFlags::PRESENT.with(PageFlags::WRITABLE);
            let dir_flags = if leaf_flags.contains(PageFlags::USER) {
                dir_flags.with(PageFlags::USER)
            } else {
                dir_flags
            };
            *pdpt_e = PageTableEntry::new(new_pd_phys, dir_flags);

            Self::flush_tlb_page(virt);
        }

        // Re-read — entry was possibly replaced above.
        let pd_phys = (*pdpt).entry(va.pdpt_idx).phys_addr();

        let pd = pd_phys as *mut PageTable;
        let pd_e = (*pd).entry_mut(va.pd_idx);
        if !pd_e.is_present() {
            return Ok(());
        }
        if pd_e.is_huge() {
            // 2 MiB → 512 × 4 KiB.
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

unsafe fn alloc_table() -> Result<u64, &'static str> {
    if !is_registry_initialized() {
        return Err("cannot allocate page table: registry not initialized");
    }
    let mut registry = global_registry_mut();
    registry
        .allocate_pages(AllocateType::AnyPages, MemoryType::AllocatedPageTable, 1)
        .inspect(|&phys| {
            let p = phys as *mut PageTable;
            (*p).zero();
        })
        .map_err(|_| "page table allocation failed")
}

/// Return phys of child table, allocating one (P|W) if `e` is empty.
unsafe fn ensure_table(e: &mut PageTableEntry) -> Result<u64, &'static str> {
    if e.is_present() {
        if e.is_huge() {
            return Err("paging: tried to walk through a huge-page entry");
        }
        return Ok(e.phys_addr());
    }

    let child_phys = alloc_table()?;
    *e = PageTableEntry::new(child_phys, PageFlags::PRESENT.with(PageFlags::WRITABLE));
    Ok(child_phys)
}
