
const USER_MMAP_BASE: u64 = 0x0000_0040_0000_0000;

/// Allocates and maps user pages; PID 0 returns ENOSYS (kernel is
/// identity-mapped, mapping over it would corrupt kernel mappings).
pub unsafe fn sys_mmap(pages: u64) -> u64 {
    if pages == 0 || pages > 4096 {
        return EINVAL;
    }
    if !crate::memory::is_registry_initialized() {
        return ENOMEM;
    }

    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    let proc = SCHEDULER.current_memory_leader_mut();

    if proc.mmap_brk == 0 {
        proc.mmap_brk = USER_MMAP_BASE;
    }

    let vaddr = proc.mmap_brk;

    // Drop registry before map_user_page — ensure_user_table re-acquires it.
    let phys = {
        let mut registry = crate::memory::global_registry_mut();
        match registry.allocate_pages(
            crate::memory::AllocateType::AnyPages,
            crate::memory::MemoryType::Allocated,
            pages,
        ) {
            Ok(addr) => addr,
            Err(_) => return ENOMEM,
        }
    };

    let flags = crate::paging::entry::PageFlags::PRESENT
        .with(crate::paging::entry::PageFlags::WRITABLE)
        .with(crate::paging::entry::PageFlags::USER)
        .with(crate::paging::entry::PageFlags::NO_EXECUTE);

    let mut ptm = crate::paging::table::PageTableManager {
        pml4_phys: proc.cr3,
    };

    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if crate::elf::map_user_page(&mut ptm, page_virt, page_phys, flags).is_err() {
            let mut registry = crate::memory::global_registry_mut();
            let _ = registry.free_pages(phys, pages);
            return ENOMEM;
        }
    }

    // Zero before exposing to user — kernel data must not leak.
    core::ptr::write_bytes(phys as *mut u8, 0, (pages * 4096) as usize);

    if proc.vma_table.insert(vaddr, phys, pages, true).is_err() {
        let mut ptm2 = crate::paging::table::PageTableManager {
            pml4_phys: proc.cr3,
        };
        for i in 0..pages {
            let _ = ptm2.unmap_4k(vaddr + i * 4096);
        }
        let mut registry = crate::memory::global_registry_mut();
        let _ = registry.free_pages(phys, pages);
        return ENOMEM;
    }

    proc.mmap_brk = vaddr + pages * 4096;
    proc.pages_allocated += pages;

    // CR3 write flushes paging-structure caches; per-page invlpg can't,
    // and intermediate COW can leave stale entries on sibling threads.
    core::arch::asm!("mov {tmp}, cr3", "mov cr3, {tmp}", tmp = out(reg) _);

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

    let mut ptm = crate::paging::table::PageTableManager {
        pml4_phys: proc.cr3,
    };
    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let _ = ptm.unmap_4k(page_virt);
    }

    if owns {
        let mut registry = crate::memory::global_registry_mut();
        let _ = registry.free_pages(phys, pages);
    }

    if proc.pages_allocated >= pages {
        proc.pages_allocated -= pages;
    }

    0
}
