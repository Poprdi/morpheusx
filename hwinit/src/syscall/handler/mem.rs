
// SYS_MMAP — allocate + map pages into user virtual address space

/// Starting virtual address for user mmap allocations.
const USER_MMAP_BASE: u64 = 0x0000_0040_0000_0000;

/// `SYS_MMAP(pages) → virt_addr`
///
/// Allocates physical pages from MemoryRegistry, maps them into the
/// calling process's address space at the next available virtual address,
/// zeroes the memory, records the mapping in the process VMA table,
/// and returns that virtual address.
///
/// Returns `-EINVAL` for bad args, `-ENOMEM` on allocation failure,
/// `-ENOSYS` for PID 0 (kernel shares identity-mapped page table).
pub unsafe fn sys_mmap(pages: u64) -> u64 {
    if pages == 0 || pages > 4096 {
        return EINVAL;
    }
    if !crate::memory::is_registry_initialized() {
        return ENOMEM;
    }

    // PID 0 uses the kernel identity-mapped page table.
    // Mapping user pages into it would corrupt kernel mappings.
    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    let proc = SCHEDULER.current_memory_leader_mut();

    // Initialize mmap_brk on first call.
    if proc.mmap_brk == 0 {
        proc.mmap_brk = USER_MMAP_BASE;
    }

    let vaddr = proc.mmap_brk;

    // Allocate physical pages.
    let mut registry = crate::memory::global_registry_mut();
    let phys = match registry.allocate_pages(
        crate::memory::AllocateType::AnyPages,
        crate::memory::MemoryType::Allocated,
        pages,
    ) {
        Ok(addr) => addr,
        Err(_) => return ENOMEM,
    };

    // Map each page into the process address space.
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
            // On failure, free what we allocated and return error.
            let _ = registry.free_pages(phys, pages);
            return ENOMEM;
        }
    }

    // Zero the memory (important for security — don't leak kernel data).
    core::ptr::write_bytes(phys as *mut u8, 0, (pages * 4096) as usize);

    // Record the mapping in the VMA table.
    if proc.vma_table.insert(vaddr, phys, pages, true).is_err() {
        // VMA table full — unmap and free.
        let mut ptm2 = crate::paging::table::PageTableManager {
            pml4_phys: proc.cr3,
        };
        for i in 0..pages {
            let _ = ptm2.unmap_4k(vaddr + i * 4096);
        }
        let _ = registry.free_pages(phys, pages);
        return ENOMEM;
    }

    proc.mmap_brk = vaddr + pages * 4096;
    proc.pages_allocated += pages;

    // ── Full TLB + paging-structure cache flush ──────────────────────
    // map_user_page calls invlpg per-page, but COW at intermediate
    // levels (PML4→PDPT→PD) can leave stale paging-structure cache
    // entries for other addresses sharing those levels.  A CR3
    // write-back flushes everything, guaranteeing any thread sharing
    // this address space sees the new mappings.
    core::arch::asm!("mov {tmp}, cr3", "mov cr3, {tmp}", tmp = out(reg) _);

    vaddr
}
// SYS_MUNMAP — unmap pages from user virtual address space

/// `SYS_MUNMAP(vaddr, pages) → 0`
///
/// Unmaps pages from the calling process's address space.
/// If the region was allocated by SYS_MMAP (owns_phys == true), the
/// physical pages are freed back to the buddy allocator.
///
/// The `vaddr` must match the exact base address of a VMA entry and
/// `pages` must match its size.  Partial unmaps are not supported.
pub unsafe fn sys_munmap(vaddr: u64, pages: u64) -> u64 {
    if vaddr == 0 || pages == 0 || pages > 1024 {
        return EINVAL;
    }
    // Ensure the address is page-aligned and in user space.
    if vaddr & 0xFFF != 0 || vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }

    // PID 0 never creates user VMAs.
    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    let proc = SCHEDULER.current_memory_leader_mut();

    // Find the VMA entry for this address.
    let (idx, vma) = match proc.vma_table.find_exact(vaddr) {
        Some(pair) => pair,
        None => return EINVAL, // not a known mapping
    };

    // Require exact size match (no partial munmap).
    if vma.pages != pages {
        return EINVAL;
    }

    let phys = vma.phys;
    let owns = vma.owns_phys;

    // Remove the VMA entry first (before any page table manipulation).
    proc.vma_table.remove(idx);

    // Unmap from the process's own page table.
    let mut ptm = crate::paging::table::PageTableManager {
        pml4_phys: proc.cr3,
    };
    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let _ = ptm.unmap_4k(page_virt);
    }

    // If we own the physical pages, free them back to the allocator.
    if owns {
        let mut registry = crate::memory::global_registry_mut();
        let _ = registry.free_pages(phys, pages);
    }

    if proc.pages_allocated >= pages {
        proc.pages_allocated -= pages;
    }

    0
}
