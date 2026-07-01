use super::state::{
    this_core_pid, ADDRESS_SPACE_LOCKS, EARLIEST_DEADLINE, PROCESS_TABLE, PROCESS_TABLE_LOCK,
    TIMED_BLOCK_COUNT,
};
use crate::hal;
use crate::process::{
    BlockReason, Process, ProcessState, MAX_PROCESSES, PROCESS_KERNEL_STACK_SIZE,
};
use core::sync::atomic::Ordering;
use morpheus_foundation::errno::{EAGAIN, ECHILD, ESRCH};
use morpheus_foundation::flags::{P_ALL, P_PID, WNOHANG};
use morpheus_foundation::types::WaitStatus;

const PAGE_SIZE: u64 = 4096;

/// Thread-group leader (process identity) of `pid`; `pid` itself if independent.
#[inline]
unsafe fn leader_of(pid: u32) -> u32 {
    PROCESS_TABLE
        .get(pid as usize)
        .and_then(|s| s.as_ref())
        .map(|p| {
            if p.is_thread() {
                p.thread_group_leader
            } else {
                pid
            }
        })
        .unwrap_or(pid)
}

/// `caller` may reap `target` iff `target` is a child of `caller`'s thread group
/// (process wait) or a sibling thread in the same group (join-from-ANY-thread).
unsafe fn can_reap(caller: u32, target: u32) -> bool {
    let caller_leader = leader_of(caller);
    let t = match PROCESS_TABLE.get(target as usize).and_then(|s| s.as_ref()) {
        Some(p) => p,
        None => return false,
    };
    let is_child = leader_of(t.parent_pid) == caller_leader;
    let is_sibling_thread = t.is_thread() && t.thread_group_leader == caller_leader;
    is_child || is_sibling_thread
}

/// Encode a finished task's Linux `wstatus` word.
#[inline]
fn encode_wstatus(proc: &Process) -> i32 {
    if proc.term_signal != 0 {
        (proc.term_signal as i32) & 0x7f
    } else {
        (proc.exit_code.unwrap_or(0) & 0xff) << 8
    }
}

pub unsafe fn block_sleep(deadline: u64) -> u64 {
    let pid = this_core_pid() as usize;
    PROCESS_TABLE_LOCK.lock();
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid) {
        proc.state = ProcessState::Blocked(BlockReason::Sleep(deadline));
        TIMED_BLOCK_COUNT.fetch_add(1, Ordering::Relaxed);
        // Race-free min update — used by tick fast-path.
        loop {
            let current_earliest = EARLIEST_DEADLINE.load(Ordering::Relaxed);
            if deadline < current_earliest {
                if EARLIEST_DEADLINE
                    .compare_exchange(
                        current_earliest,
                        deadline,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    break;
                }
            } else {
                break;
            }
        }
    }
    PROCESS_TABLE_LOCK.unlock();

    hal().cpu().halt_wait_irq();
    0
}

/// Reap a zombie slot. `None` if still executing on another core (caller retries).
/// The Zombie→Terminated transition under the table lock makes double-reap
/// (SYS_THREAD_JOIN racing SYS_WAIT on the same tid) impossible.
unsafe fn reap_zombie(pid: u32) -> Option<(i32, i32)> {
    let child = PROCESS_TABLE.get_mut(pid as usize)?.as_mut()?;
    // Don't free page tables while another core's CR3 still points at them.
    if child.running_on != u32::MAX {
        return None;
    }
    let code = child.exit_code.unwrap_or(-1);
    let wstatus = encode_wstatus(child);
    // Nonzero ⇒ this is a thread; its private stack/TLS maps live in the leader's
    // shared table, not `child.vma_table`. Captured before the borrow ends.
    let leader_pid = child.thread_group_leader;

    free_process_resources(child);
    child.state = ProcessState::Terminated;

    // Reclaim the thread's own VMAs from the leader (a full-process teardown, i.e.
    // `leader_pid == 0`, already frees every VMA in `free_process_resources`).
    if leader_pid != 0 {
        reclaim_thread_vmas(leader_pid, pid);
    }

    crate::serial::puts("[SCHED] reaped PID ");
    crate::serial::put_hex32(pid);
    crate::serial::puts("\n");

    Some((code, wstatus))
}

unsafe fn reclaim_thread_vmas(leader_pid: u32, tid: u32) {
    let lock = &ADDRESS_SPACE_LOCKS[leader_pid as usize];
    lock.lock();

    let leader = match PROCESS_TABLE
        .get_mut(leader_pid as usize)
        .and_then(|s| s.as_mut())
    {
        Some(p) => p,
        None => {
            lock.unlock();
            return;
        },
    };

    let cr3 = leader.cr3;
    let phys = hal().phys();
    let phys_ready = phys.is_initialized();
    let mut freed_pages: u64 = 0;

    leader.vma_table.drain_owner(tid, |vma| {
        for i in 0..vma.pages {
            let _ = hal().paging().pml4_unmap_4k(cr3, vma.vaddr + i * PAGE_SIZE);
        }
        if vma.owns_phys && phys_ready {
            let _ = phys.free_pages(vma.phys, vma.pages);
        }
        freed_pages += vma.pages;
    });

    if leader.pages_allocated >= freed_pages {
        leader.pages_allocated -= freed_pages;
    }
    if freed_pages != 0 {
        hal().paging().flush_tlb_all();
    }

    lock.unlock();
}

/// Auto-reap detached zombie threads whose kernel stack is free, so a dropped
/// JoinHandle doesn't leak a slot. Caller holds PROCESS_TABLE_LOCK.
pub(crate) unsafe fn reap_detached_zombies() {
    for idx in 1..MAX_PROCESSES {
        if let Some(Some(p)) = PROCESS_TABLE.get(idx) {
            if p.detached && p.state == ProcessState::Zombie && p.running_on == u32::MAX {
                let _ = reap_zombie(idx as u32);
            }
        }
    }
}

/// SYS_THREAD_JOIN(tid): block until `target` (child or sibling thread) finishes;
/// returns its exit code. Join-from-ANY-thread via the thread-group eligibility
/// check + the broad terminate-time wake.
pub unsafe fn wait_for_child(target: u32) -> u64 {
    let current = this_core_pid();

    loop {
        PROCESS_TABLE_LOCK.lock();

        let state = match PROCESS_TABLE.get(target as usize).and_then(|s| s.as_ref()) {
            Some(p) => p.state,
            None => {
                PROCESS_TABLE_LOCK.unlock();
                return ESRCH;
            },
        };

        if !can_reap(current, target) {
            PROCESS_TABLE_LOCK.unlock();
            return ESRCH;
        }

        if matches!(state, ProcessState::Terminated) {
            PROCESS_TABLE_LOCK.unlock();
            return ECHILD;
        }

        if state == ProcessState::Zombie {
            match reap_zombie(target) {
                Some((code, _)) => {
                    PROCESS_TABLE_LOCK.unlock();
                    return code as u64;
                },
                None => {
                    PROCESS_TABLE_LOCK.unlock();
                    hal().cpu().halt_wait_irq();
                    continue;
                },
            }
        }

        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(current as usize) {
            proc.state = ProcessState::Blocked(BlockReason::WaitChild(target));
        }
        PROCESS_TABLE_LOCK.unlock();
        hal().cpu().halt_wait_irq();
    }
}

/// SYS_TRY_WAIT(pid): non-blocking probe used by the compositor.
pub unsafe fn try_wait_child(target: u32) -> u64 {
    let current = this_core_pid();

    PROCESS_TABLE_LOCK.lock();

    let state = match PROCESS_TABLE.get(target as usize).and_then(|s| s.as_ref()) {
        Some(p) => p.state,
        None => {
            PROCESS_TABLE_LOCK.unlock();
            return ESRCH;
        },
    };

    if !can_reap(current, target) {
        PROCESS_TABLE_LOCK.unlock();
        return ESRCH;
    }

    if matches!(state, ProcessState::Terminated) {
        PROCESS_TABLE_LOCK.unlock();
        return ECHILD;
    }

    if state == ProcessState::Zombie {
        let r = match reap_zombie(target) {
            Some((code, _)) => code as u64,
            None => EAGAIN,
        };
        PROCESS_TABLE_LOCK.unlock();
        return r;
    }

    PROCESS_TABLE_LOCK.unlock();
    EAGAIN
}

/// SYS_WAIT(idtype, id, options) core. `value` is the reaped pid, `0` for
/// WNOHANG-with-no-ready-child, or `-errno`; `wstatus` is meaningful only when
/// `value` is a positive pid.
pub unsafe fn do_wait(idtype: u64, id: u32, options: u64) -> (u64, i32) {
    let current = this_core_pid();
    let wnohang = options & WNOHANG != 0;

    loop {
        PROCESS_TABLE_LOCK.lock();

        let mut found_any = false;
        let mut ready: Option<u32> = None;

        match idtype {
            P_PID => match PROCESS_TABLE.get(id as usize).and_then(|s| s.as_ref()) {
                Some(p)
                    if can_reap(current, id) && !matches!(p.state, ProcessState::Terminated) =>
                {
                    found_any = true;
                    if p.state == ProcessState::Zombie {
                        ready = Some(id);
                    }
                },
                _ => {},
            },
            P_ALL => {
                for idx in 1..MAX_PROCESSES {
                    let elig = match PROCESS_TABLE.get(idx).and_then(|s| s.as_ref()) {
                        Some(p) if !matches!(p.state, ProcessState::Terminated) => {
                            can_reap(current, idx as u32).then_some(p.state)
                        },
                        _ => None,
                    };
                    if let Some(st) = elig {
                        found_any = true;
                        if st == ProcessState::Zombie {
                            ready = Some(idx as u32);
                            break;
                        }
                    }
                }
            },
            // P_PGID unsupported; std uses P_PID/P_ALL.
            _ => {
                PROCESS_TABLE_LOCK.unlock();
                return (morpheus_foundation::errno::EINVAL, 0);
            },
        }

        if !found_any {
            PROCESS_TABLE_LOCK.unlock();
            return (ECHILD, 0);
        }

        if let Some(pid) = ready {
            match reap_zombie(pid) {
                Some((_, wstatus)) => {
                    PROCESS_TABLE_LOCK.unlock();
                    return (pid as u64, wstatus);
                },
                None => {
                    // Zombie still running on another core: treat as not-ready.
                    if wnohang {
                        PROCESS_TABLE_LOCK.unlock();
                        return (0, 0);
                    }
                    PROCESS_TABLE_LOCK.unlock();
                    hal().cpu().halt_wait_irq();
                    continue;
                },
            }
        }

        if wnohang {
            PROCESS_TABLE_LOCK.unlock();
            return (0, 0);
        }

        let waited = if idtype == P_PID { id } else { 0 };
        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(current as usize) {
            proc.state = ProcessState::Blocked(BlockReason::WaitChild(waited));
        }
        PROCESS_TABLE_LOCK.unlock();
        hal().cpu().halt_wait_irq();
    }
}

/// Write a `WaitStatus` to a validated user pointer; no-op if `ptr == 0`.
pub unsafe fn write_wait_status(ptr: u64, pid: u32, wstatus: i32) {
    if ptr == 0 {
        return;
    }
    let ws = WaitStatus {
        version: 1,
        struct_size: core::mem::size_of::<WaitStatus>() as u16,
        _pad0: 0,
        pid: pid as i32,
        wstatus,
        reserved: [0; 2],
    };
    core::ptr::write(ptr as *mut WaitStatus, ws);
}

unsafe fn free_process_resources(proc: &mut Process) {
    // Storage reap (spec §7): close this pid's fds (decrement per-mount open_fds)
    // and auto-umount the ephemeral (staged) mounts it owns, freeing their RAM and
    // restoring its staging budget. Direct/global mounts survive. Done first so a
    // dying process can never leak staged RAM; takes STORAGE_LOCK internally, and
    // we hold PROCESS_TABLE_LOCK here — ordering is PROCESS_TABLE_LOCK→STORAGE_LOCK
    // (no storage path takes PROCESS_TABLE_LOCK under STORAGE_LOCK).
    crate::storage::reap_process(proc.pid, &mut proc.fd_table);
    proc.fd_table = crate::storage::fs_api::FdTable::new();

    let phys = hal().phys();
    if proc.kernel_stack_base != 0 && phys.is_initialized() {
        let pages = (PROCESS_KERNEL_STACK_SIZE as u64).div_ceil(PAGE_SIZE);
        let _ = phys.free_pages(proc.kernel_stack_base, pages);
        proc.kernel_stack_base = 0;
        proc.kernel_stack_top = 0;
    }

    if proc.fb_surface_phys != 0 && proc.fb_surface_pages != 0 && phys.is_initialized() {
        let _ = phys.free_pages(proc.fb_surface_phys, proc.fb_surface_pages);
        proc.fb_surface_phys = 0;
        proc.fb_surface_pages = 0;
        proc.fb_surface_dirty = false;
    }

    if phys.is_initialized() {
        for (_, vma) in proc.vma_table.iter() {
            if vma.owns_phys {
                let _ = phys.free_pages(vma.phys, vma.pages);
            }
        }
    }

    if proc.cr3 != 0 && proc.pid != 0 && proc.thread_group_leader == 0 {
        let kernel_cr3 = hal().paging().current_cr3() & 0x000F_FFFF_FFFF_F000;

        if proc.cr3 != kernel_cr3 {
            free_user_page_tables(proc.cr3);
            proc.cr3 = 0;
        }
    }
}

/// Walk a user PML4 and free every page-table page in the lower 256 entries; free PML4 last.
///
/// ARCH-PORTABILITY-HOLE: walks raw PTEs with x86_64 bit constants (PRESENT/USER/HUGE)
/// and 4-level structure. Replace with a HAL `pml4_free_user_pages` closure.
unsafe fn free_user_page_tables(pml4_phys: u64) {
    let phys = hal().phys();
    if !phys.is_initialized() {
        return;
    }

    let pml4 = pml4_phys as *const u64;

    // ARCH-PORTABILITY-HOLE: x86_64 PTE bits.
    const PRESENT: u64 = 1 << 0;
    const USER: u64 = 1 << 2;
    const HUGE: u64 = 1 << 7;
    const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

    for pml4_idx in 0..256usize {
        let pml4e = *pml4.add(pml4_idx);
        if pml4e & PRESENT == 0 || pml4e & USER == 0 {
            continue;
        }
        let pdpt_phys = pml4e & ADDR_MASK;
        let pdpt = pdpt_phys as *const u64;

        for pdpt_idx in 0..512usize {
            let pdpte = *pdpt.add(pdpt_idx);
            if pdpte & PRESENT == 0 || pdpte & USER == 0 {
                continue;
            }
            if pdpte & HUGE != 0 {
                continue;
            }
            let pd_phys = pdpte & ADDR_MASK;
            let pd = pd_phys as *const u64;

            for pd_idx in 0..512usize {
                let pde = *pd.add(pd_idx);
                if pde & PRESENT == 0 || pde & USER == 0 {
                    continue;
                }
                if pde & HUGE != 0 {
                    continue;
                }
                let pt_phys = pde & ADDR_MASK;
                let _ = phys.free_pages(pt_phys, 1);
            }
            let _ = phys.free_pages(pd_phys, 1);
        }
        let _ = phys.free_pages(pdpt_phys, 1);
    }
    let _ = phys.free_pages(pml4_phys, 1);
}
