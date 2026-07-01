// Memory syscalls: sys_mmap / sys_munmap / sys_mprotect.

use super::common::*;
use crate::hal;
use crate::process::vma::Vma;
use crate::schedular::SCHEDULER;
use morpheus_foundation::flags::{MAP_FIXED, PROT_EXEC, PROT_NONE, PROT_WRITE};
use morpheus_hal_api::{AllocKind, MemoryType, PageFlags, Pml4Handle};

pub(crate) const USER_MMAP_BASE: u64 = 0x0000_0040_0000_0000;

/// Top of the anonymous-mmap window (512 GiB above base). Kept well below
/// `USER_ADDR_LIMIT` so a runaway reservation hits `-ENOMEM`, not a collision.
pub(crate) const USER_MMAP_LIMIT: u64 = USER_MMAP_BASE + 0x0000_0080_0000_0000;

/// Per-call page cap (1 GiB). Each mapping is one *contiguous* physical block, so
/// a single huge request would otherwise stall the buddy on a giant coalesced run.
const MMAP_MAX_PAGES: u64 = 1 << 18;

/// PROT_* bitmap → page-table preset. `PROT_NONE` (guard) wins over the access
/// bits; otherwise READ is implicit and WRITE/EXEC are bits 0/1.
pub(crate) fn prot_to_user_preset(prot: u64) -> PageFlags {
    if prot & PROT_NONE != 0 {
        return PageFlags::USER_NONE;
    }
    let w = prot & PROT_WRITE != 0;
    let x = prot & PROT_EXEC != 0;
    match (w, x) {
        (false, false) => PageFlags::USER_RO,
        (true, false) => PageFlags::USER_RW,
        (false, true) => PageFlags::USER_RX,
        (true, true) => PageFlags::USER_RWX,
    }
}

#[inline]
fn prot_bits_valid(prot: u64) -> bool {
    prot & !(PROT_WRITE | PROT_EXEC | PROT_NONE) == 0
}

/// Maps `pages` anonymous, zero-filled user pages with protection `prot`.
/// `MAP_FIXED` places the mapping at `addr`, else the kernel picks a free VA
/// (reusing freed holes). `prot == 0` defaults to RW (legacy single-arg contract).
/// PID 0 returns ENOSYS: kernel is identity-mapped, mapping over it corrupts it.
pub unsafe fn sys_mmap(pages: u64, prot: u64, flags: u64, addr: u64) -> u64 {
    if pages == 0 || pages > MMAP_MAX_PAGES {
        return EINVAL;
    }
    if !prot_bits_valid(prot) {
        return EINVAL;
    }
    if !hal().phys().is_initialized() {
        return ENOMEM;
    }
    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    let fixed = flags & MAP_FIXED != 0;
    if fixed && (addr == 0 || addr & 0xFFF != 0 || addr < USER_MMAP_BASE) {
        return EINVAL;
    }

    // Siblings share CR3; mmap_brk + vma_table + PTE edits are not atomic.
    let lock = SCHEDULER.current_address_space_lock();
    lock.lock();
    let ret = mmap_locked(pages, prot, fixed, addr);
    lock.unlock();
    ret
}

unsafe fn mmap_locked(pages: u64, prot: u64, fixed: bool, addr: u64) -> u64 {
    let proc = SCHEDULER.current_memory_leader_mut();

    let len = pages * 4096;

    let vaddr = if fixed {
        match addr.checked_add(len) {
            Some(end) if end <= USER_MMAP_LIMIT => {},
            _ => return EINVAL,
        }
        if proc.vma_table.overlaps_any(addr, pages) {
            return EINVAL; // MAP_FIXED-over-existing is not supported (no silent clobber)
        }
        addr
    } else {
        match proc
            .vma_table
            .find_free_va(USER_MMAP_BASE, USER_MMAP_LIMIT, pages)
        {
            Some(v) => v,
            None => return ENOMEM,
        }
    };

    let phys = match hal()
        .phys()
        .allocate_pages(AllocKind::AnyPages, MemoryType::Allocated, pages)
    {
        Ok(addr) => addr,
        Err(_) => return ENOMEM,
    };

    // PROT_NONE defaults: legacy/anonymous callers pass 0 and expect RW.
    let eff_prot = if prot == 0 { PROT_WRITE } else { prot };
    let preset = prot_to_user_preset(eff_prot);

    let pml4 = Pml4Handle(proc.cr3);
    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if hal()
            .paging()
            .pml4_map_user_4k(pml4, page_virt, page_phys, preset)
            .is_err()
        {
            for j in 0..i {
                let _ = hal().paging().pml4_unmap_4k(proc.cr3, vaddr + j * 4096);
            }
            let _ = hal().phys().free_pages(phys, pages);
            return ENOMEM;
        }
    }

    // Zero before exposing to user (no kernel data leak) — through the identity
    // map, independent of the user PTE protection (works for guard pages too).
    core::ptr::write_bytes(phys as *mut u8, 0, (pages * 4096) as usize);

    if proc
        .vma_table
        .insert_full(vaddr, phys, pages, true, eff_prot)
        .is_err()
    {
        for i in 0..pages {
            let _ = hal().paging().pml4_unmap_4k(proc.cr3, vaddr + i * 4096);
        }
        let _ = hal().phys().free_pages(phys, pages);
        return ENOMEM;
    }

    // Keep the MAP_PHYS bump pointer above anonymous mappings so the two VA
    // allocators never hand out the same address.
    let end = vaddr + len;
    if end > proc.mmap_brk {
        proc.mmap_brk = end;
    }
    proc.pages_allocated += pages;

    // Flush TLB on all CPUs; sibling threads share CR3 and may have stale entries.
    hal().paging().flush_tlb_all();

    vaddr
}

/// Unmaps `[vaddr, vaddr + pages)`. The range must tile whole VMAs back-to-back,
/// so a previously-split region (mmap + mprotect guard) frees in one call while
/// partial-VMA tears are rejected. Freed VAs become reusable holes.
pub unsafe fn sys_munmap(vaddr: u64, pages: u64) -> u64 {
    if vaddr == 0 || pages == 0 || pages > MMAP_MAX_PAGES {
        return EINVAL;
    }
    if vaddr & 0xFFF != 0 || vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    let lock = SCHEDULER.current_address_space_lock();
    lock.lock();
    let ret = munmap_locked(vaddr, pages);
    lock.unlock();
    ret
}

unsafe fn munmap_locked(vaddr: u64, pages: u64) -> u64 {
    let proc = SCHEDULER.current_memory_leader_mut();
    let end = vaddr + pages * 4096;

    // Verify the range is exactly tiled by VMAs before touching any PTEs, so a
    // partial/hole request fails atomically.
    let mut cursor = vaddr;
    while cursor < end {
        match proc.vma_table.find_exact(cursor) {
            Some((_, v)) if v.vaddr_end() <= end => cursor = v.vaddr_end(),
            _ => return EINVAL,
        }
    }

    let mut cursor = vaddr;
    while cursor < end {
        let idx = match proc.vma_table.find_exact(cursor) {
            Some((i, _)) => i,
            None => break,
        };
        let vma = proc.vma_table.remove(idx);
        for i in 0..vma.pages {
            let _ = hal().paging().pml4_unmap_4k(proc.cr3, vma.vaddr + i * 4096);
        }
        if vma.owns_phys {
            let _ = hal().phys().free_pages(vma.phys, vma.pages);
        }
        if proc.pages_allocated >= vma.pages {
            proc.pages_allocated -= vma.pages;
        }
        cursor = vma.vaddr_end();
    }

    hal().paging().flush_tlb_all();
    0
}

/// Changes protection of `[vaddr, vaddr + pages)`. The range must lie in a single
/// VMA, split into up to three (before/changed/after) so a sub-range — e.g. a
/// `PROT_NONE` guard page in a stack — is honored. `PROT_NONE` makes the pages
/// supervisor-only.
pub unsafe fn sys_mprotect(vaddr: u64, pages: u64, prot: u64) -> u64 {
    if pages == 0 || pages > MMAP_MAX_PAGES {
        return EINVAL;
    }
    if vaddr == 0 || vaddr & 0xFFF != 0 || vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if !prot_bits_valid(prot) {
        return EINVAL;
    }
    if SCHEDULER.current_pid() == 0 {
        return EPERM;
    }

    let lock = SCHEDULER.current_address_space_lock();
    lock.lock();
    let ret = mprotect_locked(vaddr, pages, prot);
    lock.unlock();
    ret
}

unsafe fn mprotect_locked(vaddr: u64, pages: u64, prot: u64) -> u64 {
    let proc = SCHEDULER.current_memory_leader_mut();

    let idx = match proc.vma_table.find_containing_range(vaddr, pages) {
        Some(i) => i,
        None => return ENOMEM,
    };
    let vma = proc.vma_table.get(idx);

    let end = vaddr + pages * 4096;
    let left_pages = (vaddr - vma.vaddr) / 4096;
    let right_pages = (vma.vaddr_end() - end) / 4096;

    // The reused slot becomes the middle piece; each non-empty flank needs a new
    // slot. Reject before mutating if the table cannot hold the split.
    let needed = (left_pages > 0) as usize + (right_pages > 0) as usize;
    if proc.vma_table.free_slots() < needed {
        return ENOMEM;
    }

    let preset = prot_to_user_preset(prot);
    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        if hal()
            .paging()
            .pml4_remap_flags(proc.cr3, page_virt, preset)
            .is_err()
        {
            return EFAULT;
        }
    }

    // Sub-VMAs index into the original contiguous block at their page offset.
    let mid = Vma {
        vaddr,
        phys: vma.phys + left_pages * 4096,
        pages,
        owns_phys: vma.owns_phys,
        prot,
    };
    proc.vma_table.set_at(idx, mid);

    if left_pages > 0 {
        let _ =
            proc.vma_table
                .insert_full(vma.vaddr, vma.phys, left_pages, vma.owns_phys, vma.prot);
    }
    if right_pages > 0 {
        let _ = proc.vma_table.insert_full(
            end,
            vma.phys + (left_pages + pages) * 4096,
            right_pages,
            vma.owns_phys,
            vma.prot,
        );
    }

    hal().paging().flush_tlb_all();
    0
}
