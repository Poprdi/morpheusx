// SYS_SHM_GRANT: unidirectional grant of phys frames owned by the caller into
// the target's address space. Target VMA owns_phys=false — only the granter's
// munmap frees. Caps capability: must know PID and own a matching VMA.

use super::common::*;
use super::core::sys_yield;
use super::fs::sys_fs_close;
use super::mem::USER_MMAP_BASE;
use crate::hal;
use crate::pipe;
use crate::schedular::SCHEDULER;
use morpheus_hal_api::{PageFlags, Pml4Handle};
use morpheus_helix::types::open_flags::{O_PIPE_READ, O_PIPE_WRITE};

/// Translate a PROT_{READ,WRITE,EXEC} bitmap into a `PageFlags` preset. Bit
/// values come from the IPC/mprotect syscall ABI (READ implicit on x86).
fn prot_to_user_preset(prot: u64) -> PageFlags {
    let w = prot & PROT_WRITE != 0;
    let x = prot & PROT_EXEC != 0;
    match (w, x) {
        (false, false) => PageFlags::USER_RO,
        (true, false) => PageFlags::USER_RW,
        (false, true) => PageFlags::USER_RX,
        (true, true) => PageFlags::USER_RWX,
    }
}

pub const PROT_READ: u64 = 0;
pub const PROT_WRITE: u64 = 1;
pub const PROT_EXEC: u64 = 2;

pub unsafe fn sys_shm_grant(target_pid: u64, src_vaddr: u64, pages: u64, flags: u64) -> u64 {
    let _ = sys_yield; // silence unused warning if reflow is needed.

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
    if caller_pid == 0 {
        return EPERM;
    }

    // verify source vma in the caller
    let phys = {
        let caller_proc = SCHEDULER.current_memory_leader_mut();
        let (_, src_vma) = match caller_proc.vma_table.find_exact(src_vaddr) {
            Some(pair) => pair,
            None => return EINVAL,
        };

        if src_vma.pages != pages {
            return EINVAL;
        }

        if !src_vma.owns_phys {
            return EPERM;
        }

        src_vma.phys
    };

    let target_ref = match SCHEDULER.memory_leader_mut_by_pid(target_pid as u32) {
        Some(p) => p,
        None => return ESRCH,
    };

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

    let preset = prot_to_user_preset(flags);
    let target_pml4 = Pml4Handle(target_ref.cr3);
    for i in 0..pages {
        let page_virt = target_vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if hal()
            .paging()
            .pml4_map_user_4k(target_pml4, page_virt, page_phys, preset)
            .is_err()
        {
            for j in 0..i {
                let _ = hal()
                    .paging()
                    .pml4_unmap_4k(target_ref.cr3, target_vaddr + j * 4096);
            }
            return ENOMEM;
        }
    }

    if target_ref
        .vma_table
        .insert(target_vaddr, phys, pages, false)
        .is_err()
    {
        for i in 0..pages {
            let _ = hal()
                .paging()
                .pml4_unmap_4k(target_ref.cr3, target_vaddr + i * 4096);
        }
        return ENOMEM;
    }

    target_ref.mmap_brk = target_vaddr + pages * 4096;

    target_vaddr
}

/// Flip PTE flags on an existing VMA (exact match, no splits).
/// prot: bit0 PROT_WRITE, bit1 PROT_EXEC. Read is implicit on x86-64.
pub unsafe fn sys_mprotect(vaddr: u64, pages: u64, prot: u64) -> u64 {
    if pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if vaddr == 0 || vaddr & 0xFFF != 0 {
        return EINVAL;
    }
    if vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if prot & !3 != 0 {
        return EINVAL;
    }

    if SCHEDULER.current_pid() == 0 {
        return EPERM;
    }

    let proc = SCHEDULER.current_memory_leader_mut();

    let (_, vma) = match proc.vma_table.find_exact(vaddr) {
        Some(pair) => pair,
        None => return EINVAL,
    };

    if vma.pages != pages {
        return EINVAL;
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

    0
}

/// Writes `[read_fd, write_fd]` (two u64) at `result_ptr`.
pub unsafe fn sys_pipe(result_ptr: u64) -> u64 {
    if !validate_user_buf(result_ptr, 8) {
        return EFAULT;
    }
    let pipe_idx = match pipe::pipe_alloc() {
        Some(idx) => idx,
        None => return ENOMEM,
    };

    let fd_table = SCHEDULER.current_fd_table_mut();

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

    let out = result_ptr as *mut [u32; 2];
    (*out)[0] = read_fd as u32;
    (*out)[1] = write_fd as u32;
    0
}

/// Silently closes `new_fd` first if it was open.
pub unsafe fn sys_dup2(old_fd: u64, new_fd: u64) -> u64 {
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

    if fd_table.get(new_fd as usize).is_ok() {
        sys_fs_close(new_fd);
    }

    let fd_table = SCHEDULER.current_fd_table_mut();
    if new_fd as usize >= morpheus_helix::types::MAX_FDS {
        return EBADF;
    }

    fd_table.fds[new_fd as usize] = src;

    if src.flags & O_PIPE_READ != 0 {
        pipe::pipe_add_reader(src.mount_idx);
    }
    if src.flags & O_PIPE_WRITE != 0 {
        pipe::pipe_add_writer(src.mount_idx);
    }

    new_fd
}

// SYS_SET_FG
pub unsafe fn sys_set_fg(pid: u64) -> u64 {
    crate::stdin::set_foreground_pid(pid as u32);
    0
}

/// Copies NUL-separated argv blob; returns argc.
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

/// Blocks until data, or returns 0 once all writers have closed.
pub(super) unsafe fn sys_pipe_read_blocking(pipe_idx: u8, buf: &mut [u8]) -> u64 {
    loop {
        let n = pipe::pipe_read(pipe_idx, buf);
        if n > 0 {
            return n as u64;
        }
        if pipe::pipe_writers(pipe_idx) == 0 {
            return 0;
        }
        {
            let proc = SCHEDULER.current_process_mut();
            crate::schedular::mark_pipe_waiter(proc.pid, pipe_idx);
            proc.state = crate::process::ProcessState::Blocked(
                crate::process::BlockReason::PipeRead(pipe_idx),
            );
        }
        hal().cpu().halt_wait_irq();
    }
}
