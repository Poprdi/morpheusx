// SYS_SHM_GRANT (73) — grant shared physical pages to another process
//
// Exokernel shared memory primitive.  The caller specifies physical pages
// it owns (via SYS_MMAP or SYS_DMA_ALLOC), and the kernel maps those same
// physical frames into the target process's address space.
//
// This is *unidirectional grant*, not symmetric attach.  The granting
// process retains its own mapping.  The target receives a new VMA with
// `owns_phys = false` so that munmap in the target does NOT free the
// physical pages (the granter still owns them).
//
// # Arguments
//
//   a1 = target_pid (u32)
//   a2 = source virtual address (must be start of a VMA in the caller)
//   a3 = number of 4 KiB pages (must match the VMA exactly)
//   a4 = flags: bit 0 = writable, bit 1 = executable
//
// # Returns
//
//   Virtual address in the target process, or error code.
//
// # Security model
//
//   - Only processes that OWN physical pages can grant them (owns_phys=true)
//   - The target process cannot free the underlying physical memory
//   - The granter can munmap its side, but the target's mapping persists
//     until the target munmaps or exits
//   - There is no ambient authority: you must know the PID and possess
//     a valid VMA

/// Protection flags for SYS_SHM_GRANT and SYS_MPROTECT.
pub const PROT_READ: u64 = 0; // Read-only (no additional bits)
pub const PROT_WRITE: u64 = 1; // Writable
pub const PROT_EXEC: u64 = 2; // Executable (clears NX)

/// `SYS_SHM_GRANT(target_pid, src_vaddr, pages, flags) → target_vaddr`
pub unsafe fn sys_shm_grant(target_pid: u64, src_vaddr: u64, pages: u64, flags: u64) -> u64 {
    // argument validation
    if pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if src_vaddr == 0 || src_vaddr & 0xFFF != 0 {
        return EINVAL;
    }
    if src_vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }

    let caller_pid = SCHEDULER.current_pid();

    // Cannot grant to self (use mmap), cannot grant to kernel.
    if target_pid == 0 || target_pid == caller_pid as u64 {
        return EINVAL;
    }
    // Caller must not be the kernel.
    if caller_pid == 0 {
        return EPERM;
    }

    // verify source vma in the caller
    let phys = {
        let caller_proc = SCHEDULER.current_memory_leader_mut();
        let (_, src_vma) = match caller_proc.vma_table.find_exact(src_vaddr) {
            Some(pair) => pair,
            None => return EINVAL, // not a known mapping
        };

        // Must match exact page count.
        if src_vma.pages != pages {
            return EINVAL;
        }

        // Only owned physical pages can be granted.  We refuse to re-grant
        // pages that were themselves granted (owns_phys=false), because the
        // original owner controls their lifetime.
        if !src_vma.owns_phys {
            return EPERM;
        }

        src_vma.phys
    };

    // compute target virtual address
    // We need mutable access to the target.  Re-acquire via memory leader
    // access since we just dropped the caller reference.
    // SAFETY: single-core, interrupts disabled during syscall.
    let target_ref = match SCHEDULER.memory_leader_mut_by_pid(target_pid as u32) {
        Some(p) => p,
        None => return ESRCH,
    };

    // Target must be alive (Ready, Running, or Blocked).
    if target_ref.is_free() {
        return ESRCH;
    }
    if target_ref.cr3 == 0 {
        return ESRCH; // kernel thread without user page table
    }

    if target_ref.mmap_brk == 0 {
        target_ref.mmap_brk = USER_MMAP_BASE;
    }
    let target_vaddr = target_ref.mmap_brk;

    // build pte flags
    let mut pte_flags = crate::paging::entry::PageFlags::PRESENT
        .with(crate::paging::entry::PageFlags::USER)
        .with(crate::paging::entry::PageFlags::NO_EXECUTE);

    if flags & PROT_WRITE != 0 {
        pte_flags = pte_flags.with(crate::paging::entry::PageFlags::WRITABLE);
    }
    if flags & PROT_EXEC != 0 {
        pte_flags = pte_flags.without(crate::paging::entry::PageFlags::NO_EXECUTE);
    }

    // map physical pages into target's address space
    let mut ptm = crate::paging::table::PageTableManager {
        pml4_phys: target_ref.cr3,
    };

    for i in 0..pages {
        let page_virt = target_vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if crate::elf::map_user_page(&mut ptm, page_virt, page_phys, pte_flags).is_err() {
            // Roll back: unmap pages we already mapped.
            let mut ptm2 = crate::paging::table::PageTableManager {
                pml4_phys: target_ref.cr3,
            };
            for j in 0..i {
                let _ = ptm2.unmap_4k(target_vaddr + j * 4096);
            }
            return ENOMEM;
        }
    }

    // record vma in target (owns_phys = false)
    if target_ref
        .vma_table
        .insert(target_vaddr, phys, pages, false)
        .is_err()
    {
        // VMA table full — unmap everything.
        let mut ptm3 = crate::paging::table::PageTableManager {
            pml4_phys: target_ref.cr3,
        };
        for i in 0..pages {
            let _ = ptm3.unmap_4k(target_vaddr + i * 4096);
        }
        return ENOMEM;
    }

    target_ref.mmap_brk = target_vaddr + pages * 4096;

    target_vaddr
}

// SYS_MPROTECT (74) — change page protection flags
//
// Modifies the x86-64 page table flags on an existing VMA in the calling
// process.  This is a bare page-table-flag-flip — the minimum kernel
// mechanism for W^X enforcement, guard pages, and JIT compilation.
//
// # Arguments
//
//   a1 = virtual address (must be the exact start of a VMA)
//   a2 = number of 4 KiB pages (must match the VMA exactly)
//   a3 = protection flags:
//        bit 0 (PROT_WRITE) = set WRITABLE
//        bit 1 (PROT_EXEC)  = clear NO_EXECUTE (allow execution)
//        All other bits must be zero.
//        PROT_READ is implied (a present page is always readable on x86-64).
//
// # Returns
//
//   0 on success, or error code.
//
// # Constraints
//
//   - Must match an existing VMA exactly (vaddr and pages).
//   - PROT_WRITE | PROT_EXEC simultaneously is allowed but discouraged
//     (breaks W^X).
//   - Does NOT split VMAs.  If you need different protections on sub-ranges,
//     mmap separate regions.

/// `SYS_MPROTECT(vaddr, pages, prot) → 0`
pub unsafe fn sys_mprotect(vaddr: u64, pages: u64, prot: u64) -> u64 {
    // argument validation
    if pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if vaddr == 0 || vaddr & 0xFFF != 0 {
        return EINVAL;
    }
    if vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    // Only bits 0 and 1 are defined.
    if prot & !3 != 0 {
        return EINVAL;
    }

    if SCHEDULER.current_pid() == 0 {
        return EPERM;
    }

    let proc = SCHEDULER.current_memory_leader_mut();

    // find the vma
    let (_, vma) = match proc.vma_table.find_exact(vaddr) {
        Some(pair) => pair,
        None => return EINVAL,
    };

    if vma.pages != pages {
        return EINVAL;
    }

    // build new pte flags
    // Base: PRESENT + USER + NX (read-only, non-executable)
    let mut new_flags = crate::paging::entry::PageFlags::PRESENT
        .with(crate::paging::entry::PageFlags::USER)
        .with(crate::paging::entry::PageFlags::NO_EXECUTE);

    if prot & PROT_WRITE != 0 {
        new_flags = new_flags.with(crate::paging::entry::PageFlags::WRITABLE);
    }
    if prot & PROT_EXEC != 0 {
        new_flags = new_flags.without(crate::paging::entry::PageFlags::NO_EXECUTE);
    }

    // walk and update each pte
    //
    // We walk the process's own page table tree.  Each 4 KiB page maps
    // to a leaf PTE at the PT level.  We rewrite the PTE preserving the
    // physical address but replacing the flag bits.
    //
    // This is safe because:
    //   1. We verified the VMA exists (so the pages ARE mapped).
    //   2. We only touch leaf PTEs in the user's page table.
    //   3. We flush the TLB after every PTE write.

    let pml4 = proc.cr3 as *mut crate::paging::entry::PageTable;

    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let va = crate::paging::table::VirtAddr::from_u64(page_virt);

        // Walk PML4 → PDPT → PD → PT
        let pml4_e = (*pml4).entry(va.pml4_idx);
        if !pml4_e.is_present() {
            return EFAULT; // page table corruption — shouldn't happen
        }

        let pdpt = pml4_e.phys_addr() as *mut crate::paging::entry::PageTable;
        let pdpt_e = (*pdpt).entry(va.pdpt_idx);
        if !pdpt_e.is_present() {
            return EFAULT;
        }

        let pd = pdpt_e.phys_addr() as *mut crate::paging::entry::PageTable;
        let pd_e = (*pd).entry(va.pd_idx);
        if !pd_e.is_present() {
            return EFAULT;
        }
        if pd_e.is_huge() {
            // 2 MiB huge page — cannot mprotect sub-ranges of a huge page.
            // This shouldn't occur for user VMAs (we only map 4 KiB pages).
            return EINVAL;
        }

        let pt = pd_e.phys_addr() as *mut crate::paging::entry::PageTable;
        let pte = (*pt).entry_mut(va.pt_idx);

        if !pte.is_present() {
            return EFAULT; // VMA says it's mapped but PTE disagrees
        }

        // Preserve the physical address, replace flags.
        let phys_addr = pte.phys_addr();
        *pte = crate::paging::entry::PageTableEntry::new(phys_addr, new_flags);

        crate::paging::table::PageTableManager::flush_tlb_page(page_virt);
    }

    0
}

// SYS_PIPE (75) — create a unidirectional pipe

/// `SYS_PIPE(result_ptr) → 0`
///
/// Creates a pipe.  Writes `[read_fd, write_fd]` (two u64s) at `result_ptr`.
pub unsafe fn sys_pipe(result_ptr: u64) -> u64 {
    if !validate_user_buf(result_ptr, 8) {
        return EFAULT;
    }
    let pipe_idx = match crate::pipe::pipe_alloc() {
        Some(idx) => idx,
        None => return ENOMEM,
    };

    let fd_table = SCHEDULER.current_fd_table_mut();

    // Allocate read-end fd.
    let read_fd = match fd_table.alloc() {
        Ok(fd) => fd,
        Err(_) => return ENOMEM,
    };
    fd_table.fds[read_fd] = morpheus_helix::types::FileDescriptor {
        key: 0,
        path: [0u8; 256],
        flags: O_PIPE_READ,
        offset: 0,
        mount_idx: pipe_idx,
        _pad: [0; 3],
        pinned_lsn: 0,
    };

    // Allocate write-end fd.
    let write_fd = match fd_table.alloc() {
        Ok(fd) => fd,
        Err(_) => {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, read_fd);
            return ENOMEM;
        }
    };
    fd_table.fds[write_fd] = morpheus_helix::types::FileDescriptor {
        key: 0,
        path: [0u8; 256],
        flags: O_PIPE_WRITE,
        offset: 0,
        mount_idx: pipe_idx,
        _pad: [0; 3],
        pinned_lsn: 0,
    };

    // Write back as [u32; 2] — matches userspace `fds: [u32; 2]`.
    let out = result_ptr as *mut [u32; 2];
    (*out)[0] = read_fd as u32;
    (*out)[1] = write_fd as u32;
    0
}

// SYS_DUP2 (76) — duplicate a file descriptor

/// `SYS_DUP2(old_fd, new_fd) → new_fd`
///
/// Duplicate `old_fd` into `new_fd`.  If `new_fd` is already open it is
/// silently closed first.
pub unsafe fn sys_dup2(old_fd: u64, new_fd: u64) -> u64 {
    // POSIX: dup2(a, a) is a safe no-op. We handle it anyway.
    if old_fd == new_fd {
        let fd_table = SCHEDULER.current_fd_table_mut();
        return if fd_table.get(old_fd as usize).is_ok() {
            new_fd
        } else {
            EBADF
        };
    }

    let fd_table = SCHEDULER.current_fd_table_mut();
    let src = match fd_table.get(old_fd as usize) {
        Ok(d) => *d,
        Err(_) => return EBADF,
    };

    // Close new_fd if it's already in use (pipe-aware, everything gets cleaned up).
    if fd_table.get(new_fd as usize).is_ok() {
        sys_fs_close(new_fd);
    }

    // Ensure new_fd slot is within bounds.
    let fd_table = SCHEDULER.current_fd_table_mut();
    if new_fd as usize >= morpheus_helix::types::MAX_FDS {
        return EBADF;
    }

    // Place the duplicated descriptor (now we have two fds to the same underlying resource).
    fd_table.fds[new_fd as usize] = src;

    // Bump pipe refcounts. If the source is a pipe, we now have an additional
    // reader/writer for that pipe. Close either one and the pipe stays alive
    // as long as the other reader/writer exists. You're welcome, POSIX semantics.
    if src.flags & O_PIPE_READ != 0 {
        crate::pipe::pipe_add_reader(src.mount_idx);
    }
    if src.flags & O_PIPE_WRITE != 0 {
        crate::pipe::pipe_add_writer(src.mount_idx);
    }

    new_fd
}

// SYS_SET_FG (77) — set foreground process for stdin

/// `SYS_SET_FG(pid) → 0`
pub unsafe fn sys_set_fg(pid: u64) -> u64 {
    crate::stdin::set_foreground_pid(pid as u32);
    0
}

// SYS_GETARGS (78) — retrieve command-line arguments

/// `SYS_GETARGS(buf_ptr, buf_len) → argc`
///
/// Copies the null-separated argument blob into the user buffer.
/// Returns the argument count (argc) in RAX.
pub unsafe fn sys_getargs(buf_ptr: u64, buf_len: u64) -> u64 {
    let proc = SCHEDULER.current_process_mut();
    let argc = proc.argc;
    let args_len = proc.args_len as usize;

    if buf_ptr != 0 && buf_len > 0 {
        let copy_len = core::cmp::min(args_len, buf_len as usize);
        if validate_user_buf(buf_ptr, copy_len as u64) {
            let dst = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len);
            dst.copy_from_slice(&proc.args[..copy_len]);
        }
    }

    argc as u64
}

// Helper — blocking pipe read

/// Read from a pipe, blocking if empty until data arrives or all writers close.
unsafe fn sys_pipe_read_blocking(pipe_idx: u8, buf: &mut [u8]) -> u64 {
    loop {
        let n = crate::pipe::pipe_read(pipe_idx, buf);
        if n > 0 {
            return n as u64;
        }
        // No data — if no writers remain, return EOF (0).
        if crate::pipe::pipe_writers(pipe_idx) == 0 {
            return 0;
        }
        // Block until a writer wakes us.
        {
            let proc = SCHEDULER.current_process_mut();
            proc.state = crate::process::ProcessState::Blocked(
                crate::process::BlockReason::PipeRead(pipe_idx),
            );
        }
        core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
    }
}
