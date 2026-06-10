// Memory syscalls: sys_mmap / sys_munmap.

use super::common::*;
use crate::hal;
use crate::schedular::SCHEDULER;
use morpheus_foundation::PAGE_SIZE;
use morpheus_hal_api::{AllocKind, MemoryType, PageFlags, Pml4Handle};

pub(crate) const USER_MMAP_BASE: u64 = 0x0000_0040_0000_0000;

/// Allocates and maps user pages; PID 0 returns ENOSYS (kernel is
/// identity-mapped, mapping over it would corrupt kernel mappings).
pub unsafe fn sys_mmap(pages: u64) -> u64 {
    if pages == 0 || pages > 4096 {
        return EINVAL;
    }
    if !hal().phys().is_initialized() {
        return ENOMEM;
    }

    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    // Serialize against sibling threads sharing this address space (shared CR3):
    // mmap_brk + vma_table + page-table edits are not otherwise atomic, so two
    // concurrent mmaps would hand back the same vaddr and a munmap of one would
    // tear down the page the other is mid-write on.
    let lock = SCHEDULER.current_address_space_lock();
    lock.lock();
    let ret = mmap_locked(pages);
    lock.unlock();
    ret
}

unsafe fn mmap_locked(pages: u64) -> u64 {
    let proc = SCHEDULER.current_memory_leader_mut();

    if proc.mmap_brk == 0 {
        proc.mmap_brk = USER_MMAP_BASE;
    }

    let vaddr = proc.mmap_brk;

    // Drop registry before map_user_page — ensure_user_table re-acquires it.
    let phys = match hal()
        .phys()
        .allocate_pages(AllocKind::AnyPages, MemoryType::Allocated, pages)
    {
        Ok(addr) => addr,
        Err(_) => return ENOMEM,
    };

    // SYS_MMAP semantics: writable, user, NX (data only).
    let pml4 = Pml4Handle(proc.cr3);
    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if hal()
            .paging()
            .pml4_map_user_4k(pml4, page_virt, page_phys, PageFlags::USER_RW)
            .is_err()
        {
            let _ = hal().phys().free_pages(phys, pages);
            return ENOMEM;
        }
    }

    // Zero before exposing to user — kernel data must not leak.
    core::ptr::write_bytes(phys as *mut u8, 0, (pages * 4096) as usize);

    if proc.vma_table.insert(vaddr, phys, pages, true).is_err() {
        for i in 0..pages {
            let _ = hal().paging().pml4_unmap_4k(proc.cr3, vaddr + i * 4096);
        }
        let _ = hal().phys().free_pages(phys, pages);
        return ENOMEM;
    }

    proc.mmap_brk = vaddr + pages * 4096;
    proc.pages_allocated += pages;

    // CR3 reload flushes paging-structure caches; intermediate COW can leave
    // stale entries on sibling threads. `flush_tlb_all` covers this on every
    // arch the HAL serves.
    hal().paging().flush_tlb_all();

    vaddr
}

/// `vaddr` and `pages` must match a VMA exactly — no partial unmap.
/// Frees physical pages if the VMA owns them.
pub unsafe fn sys_munmap(vaddr: u64, pages: u64) -> u64 {
    if vaddr == 0 || pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if vaddr & 0xFFF != 0 || vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }

    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    // Same per-address-space lock as mmap — see sys_mmap.
    let lock = SCHEDULER.current_address_space_lock();
    lock.lock();
    let ret = munmap_locked(vaddr, pages);
    lock.unlock();
    ret
}

unsafe fn munmap_locked(vaddr: u64, pages: u64) -> u64 {
    let proc = SCHEDULER.current_memory_leader_mut();

    let (idx, vma) = match proc.vma_table.find_exact(vaddr) {
        Some(pair) => pair,
        None => return EINVAL,
    };

    if vma.pages != pages {
        return EINVAL;
    }

    let phys = vma.phys;
    let owns = vma.owns_phys;

    // Drop VMA entry first so a partial failure can't double-free.
    proc.vma_table.remove(idx);

    for i in 0..pages {
        let _ = hal().paging().pml4_unmap_4k(proc.cr3, vaddr + i * 4096);
    }

    if owns {
        let _ = hal().phys().free_pages(phys, pages);
    }

    if proc.pages_allocated >= pages {
        proc.pages_allocated -= pages;
    }

    let _ = PAGE_SIZE; // touched only to keep the import warning-free for readers.

    0
}
