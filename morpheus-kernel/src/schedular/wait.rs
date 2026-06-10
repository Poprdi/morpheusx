use super::state::{
    this_core_pid, EARLIEST_DEADLINE, PROCESS_TABLE, PROCESS_TABLE_LOCK, TIMED_BLOCK_COUNT,
};
use crate::hal;
use crate::process::{BlockReason, Process, ProcessState, PROCESS_KERNEL_STACK_SIZE};
use core::sync::atomic::Ordering;
use morpheus_foundation::errno::{EAGAIN, ECHILD, ESRCH};

const PAGE_SIZE: u64 = 4096;

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

pub unsafe fn wait_for_child(child_pid: u32) -> u64 {
    let current = this_core_pid();

    loop {
        PROCESS_TABLE_LOCK.lock();

        let (child_parent, child_state) = match PROCESS_TABLE
            .get(child_pid as usize)
            .and_then(|s| s.as_ref())
        {
            Some(p) => (p.parent_pid, p.state),
            None => {
                PROCESS_TABLE_LOCK.unlock();
                return ESRCH;
            },
        };

        if child_parent != current {
            PROCESS_TABLE_LOCK.unlock();
            return ESRCH;
        }

        if matches!(child_state, ProcessState::Terminated) {
            PROCESS_TABLE_LOCK.unlock();
            return ECHILD;
        }

        if child_state == ProcessState::Zombie {
            let result = reap_child(child_pid);
            PROCESS_TABLE_LOCK.unlock();
            // Reap deferred — child is Zombie but still on another core's CPU.
            if result == EAGAIN {
                hal().cpu().halt_wait_irq();
                continue;
            }
            return result;
        }

        let cur = current as usize;
        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(cur) {
            proc.state = ProcessState::Blocked(BlockReason::WaitChild(child_pid));
        }

        PROCESS_TABLE_LOCK.unlock();

        hal().cpu().halt_wait_irq();
    }
}

pub unsafe fn try_wait_child(child_pid: u32) -> u64 {
    let current = this_core_pid();

    PROCESS_TABLE_LOCK.lock();

    let (child_parent, child_state) = match PROCESS_TABLE
        .get(child_pid as usize)
        .and_then(|s| s.as_ref())
    {
        Some(p) => (p.parent_pid, p.state),
        None => {
            PROCESS_TABLE_LOCK.unlock();
            return ESRCH;
        },
    };

    if child_parent != current {
        PROCESS_TABLE_LOCK.unlock();
        return ESRCH;
    }

    if matches!(child_state, ProcessState::Terminated) {
        PROCESS_TABLE_LOCK.unlock();
        return ECHILD;
    }

    if child_state == ProcessState::Zombie {
        let result = reap_child(child_pid);
        PROCESS_TABLE_LOCK.unlock();
        return result;
    }

    PROCESS_TABLE_LOCK.unlock();
    EAGAIN
}

unsafe fn reap_child(pid: u32) -> u64 {
    if let Some(Some(child)) = PROCESS_TABLE.get_mut(pid as usize) {
        // Don't free page tables while another core's CR3 still points at them.
        if child.running_on != u32::MAX {
            return EAGAIN;
        }

        let code = child.exit_code.unwrap_or(-1);

        free_process_resources(child);

        child.state = ProcessState::Terminated;

        crate::serial::puts("[SCHED] reaped PID ");
        crate::serial::put_hex32(pid);
        crate::serial::puts("\n");

        code as u64
    } else {
        ECHILD
    }
}

unsafe fn free_process_resources(proc: &mut Process) {
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

/// Walk a user PML4 and free every page-table page reachable from the
/// user-half (lower 256 entries). Free the PML4 itself last.
///
/// # Portability hole
/// This walks raw u64 PTEs using x86_64-shaped bit constants (PRESENT @ 0,
/// USER @ 2, HUGE @ 7) and 4-level structure (PML4 → PDPT → PD → PT). A
/// proper HAL method (`pml4_free_user_pages` taking a closure) should
/// eventually replace this — tracked as a follow-up after K8/K9 land.
/// Marked here so the audit grep can find it.
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
