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
pub use morpheus_foundation::flags::{PROT_EXEC, PROT_READ, PROT_WRITE};
use morpheus_hal_api::Pml4Handle;
use morpheus_helix::types::open_flags::{O_PIPE_READ, O_PIPE_WRITE};

// PROT_* preset lives with the VM syscalls; shm_grant reuses it.
use super::mem::prot_to_user_preset;

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

    if target_pid == 0 || target_pid == caller_pid as u64 {
        return EINVAL;
    }
    if caller_pid == 0 {
        return EPERM;
    }
    if target_pid >= crate::process::MAX_PROCESSES as u64 {
        return ESRCH;
    }

    // Acquire both address-space locks in leader-pid order to prevent deadlock on
    // concurrent reverse grants; equal leaders collapse to one lock.
    let caller_leader = SCHEDULER.current_memory_leader_pid();
    let target_leader = SCHEDULER.memory_leader_pid_of(target_pid as u32);
    let lo = caller_leader.min(target_leader);
    let hi = caller_leader.max(target_leader);
    let lock_lo = SCHEDULER.address_space_lock(lo);
    lock_lo.lock();
    let lock_hi = if hi != lo {
        let l = SCHEDULER.address_space_lock(hi);
        l.lock();
        Some(l)
    } else {
        None
    };

    let ret = shm_grant_locked(target_pid, src_vaddr, pages, flags);

    if let Some(l) = lock_hi {
        l.unlock();
    }
    lock_lo.unlock();
    ret
}

unsafe fn shm_grant_locked(target_pid: u64, src_vaddr: u64, pages: u64, flags: u64) -> u64 {
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
        return ESRCH; // kernel thread; no user page table
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

    // Pipe fds: no mount; pipe index lives in `mount_id` field.
    let read_fd = match fd_table.alloc() {
        Some(fd) => fd,
        None => return ENOMEM,
    };
    let mut read_state = crate::storage::fs_api::FdState::empty();
    read_state.flags = O_PIPE_READ;
    read_state.mount_id = pipe_idx as u64;
    if !fd_table.set(read_fd, read_state) {
        return ENOMEM;
    }

    let write_fd = match fd_table.alloc() {
        Some(fd) => fd,
        None => {
            let _ = fd_table.free(read_fd);
            return ENOMEM;
        },
    };
    let mut write_state = crate::storage::fs_api::FdState::empty();
    write_state.flags = O_PIPE_WRITE;
    write_state.mount_id = pipe_idx as u64;
    if !fd_table.set(write_fd, write_state) {
        let _ = fd_table.free(read_fd);
        return ENOMEM;
    }

    let out = result_ptr as *mut [u32; 2];
    (*out)[0] = read_fd as u32;
    (*out)[1] = write_fd as u32;
    0
}

/// Silently closes `new_fd` first if it was open.
pub unsafe fn sys_dup2(old_fd: u64, new_fd: u64) -> u64 {
    if old_fd == new_fd {
        let fd_table = SCHEDULER.current_fd_table_mut();
        return if fd_table.get(old_fd as usize).is_some() {
            new_fd
        } else {
            EBADF
        };
    }

    let fd_table = SCHEDULER.current_fd_table_mut();
    let src = match fd_table.get(old_fd as usize) {
        Some(d) => *d,
        None => return EBADF,
    };

    if fd_table.get(new_fd as usize).is_some() {
        sys_fs_close(new_fd);
    }

    let fd_table = SCHEDULER.current_fd_table_mut();
    if new_fd as usize >= crate::storage::fs_api::FD_TABLE_LEN {
        return EBADF;
    }

    if !fd_table.set(new_fd as usize, src) {
        return EBADF;
    }

    let pipe_idx = src.mount_id as u8;
    if src.flags & O_PIPE_READ != 0 {
        pipe::pipe_add_reader(pipe_idx);
    }
    if src.flags & O_PIPE_WRITE != 0 {
        pipe::pipe_add_writer(pipe_idx);
    }

    new_fd
}

// SYS_SET_FG
pub unsafe fn sys_set_fg(pid: u64) -> u64 {
    crate::stdin::set_foreground_pid(pid as u32);
    0
}

/// `buf == 0` returns argc; otherwise copies the NUL-separated argv blob and returns bytes written.
/// Must return bytes, not argc: callers slice `buf[..ret]`.
pub unsafe fn sys_getargs(buf_ptr: u64, buf_len: u64) -> u64 {
    let proc = SCHEDULER.current_process_mut();
    let argc = proc.argc;
    let args_len = proc.args_len as usize;

    if buf_ptr == 0 || buf_len == 0 {
        return argc as u64;
    }

    let copy_len = core::cmp::min(args_len, buf_len as usize);
    if copy_len == 0 || !validate_user_buf(buf_ptr, copy_len as u64) {
        return 0;
    }
    let dst = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len);
    dst.copy_from_slice(&proc.args[..copy_len]);
    copy_len as u64
}

/// SYS_GETENV: `buf_ptr,buf_len -> total_block_bytes | -errno`. NUL-separated
/// `KEY=VALUE` records like SYS_GETARGS. `buf == 0` probes the total size; a short
/// buffer copies a prefix but still returns the full size. Env lives on the
/// thread-group leader so threads share the process environ.
pub unsafe fn sys_getenv(buf_ptr: u64, buf_len: u64) -> u64 {
    let leader = SCHEDULER.current_memory_leader_mut();
    let total = leader.env_block.len();

    if buf_ptr == 0 || buf_len == 0 {
        return total as u64;
    }

    let copy_len = core::cmp::min(total, buf_len as usize);
    if copy_len > 0 {
        if !validate_user_buf(buf_ptr, copy_len as u64) {
            return EFAULT;
        }
        let dst = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len);
        dst.copy_from_slice(&leader.env_block[..copy_len]);
    }
    total as u64
}

/// Returns 0 when all writers have closed.
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
