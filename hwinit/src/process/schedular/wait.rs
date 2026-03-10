use super::state::{
    this_core_pid, EARLIEST_DEADLINE, PROCESS_TABLE, PROCESS_TABLE_LOCK, TIMED_BLOCK_COUNT,
};
use crate::process::{BlockReason, Process, ProcessState, PROCESS_KERNEL_STACK_SIZE};
use crate::memory::{global_registry_mut, is_registry_initialized, PAGE_SIZE};
use core::sync::atomic::Ordering;

pub unsafe fn block_sleep(deadline: u64) -> u64 {
    let pid = this_core_pid() as usize;
    PROCESS_TABLE_LOCK.lock();
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid) {
        proc.state = ProcessState::Blocked(BlockReason::Sleep(deadline));
        TIMED_BLOCK_COUNT.fetch_add(1, Ordering::Relaxed);
        // update earliest deadline for fast wake-path early exit
        loop {
            let current_earliest = EARLIEST_DEADLINE.load(Ordering::Relaxed);
            if deadline < current_earliest {
                if EARLIEST_DEADLINE.compare_exchange(current_earliest, deadline, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                    break;
                }
            } else {
                break;
            }
        }
    }
    PROCESS_TABLE_LOCK.unlock();

    core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
    0
}

pub unsafe fn wait_for_child(child_pid: u32) -> u64 {
    let current = this_core_pid();

    PROCESS_TABLE_LOCK.lock();

    let (child_parent, child_state) = match PROCESS_TABLE
        .get(child_pid as usize)
        .and_then(|s| s.as_ref())
    {
        Some(p) => (p.parent_pid, p.state),
        None => {
            PROCESS_TABLE_LOCK.unlock();
            return u64::MAX - 3;
        }
    };

    if child_parent != current {
        PROCESS_TABLE_LOCK.unlock();
        return u64::MAX - 3;
    }

    if matches!(child_state, ProcessState::Terminated) {
        PROCESS_TABLE_LOCK.unlock();
        return u64::MAX - 10;
    }

    if child_state == ProcessState::Zombie {
        let result = reap_child(child_pid);
        PROCESS_TABLE_LOCK.unlock();
        return result;
    }

    let cur = current as usize;
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(cur) {
        proc.state = ProcessState::Blocked(BlockReason::WaitChild(child_pid));
    }

    PROCESS_TABLE_LOCK.unlock();

    core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));

    PROCESS_TABLE_LOCK.lock();
    let result = reap_child(child_pid);
    PROCESS_TABLE_LOCK.unlock();
    result
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
            return u64::MAX - 3;
        }
    };

    if child_parent != current {
        PROCESS_TABLE_LOCK.unlock();
        return u64::MAX - 3;
    }

    if matches!(child_state, ProcessState::Terminated) {
        PROCESS_TABLE_LOCK.unlock();
        return u64::MAX - 10;
    }

    if child_state == ProcessState::Zombie {
        let result = reap_child(child_pid);
        PROCESS_TABLE_LOCK.unlock();
        return result;
    }

    PROCESS_TABLE_LOCK.unlock();
    u64::MAX - 11
}

unsafe fn reap_child(pid: u32) -> u64 {
    if let Some(Some(child)) = PROCESS_TABLE.get_mut(pid as usize) {
        let code = child.exit_code.unwrap_or(-1);

        free_process_resources(child);

        child.state = ProcessState::Terminated;

        crate::serial::puts("[SCHED] reaped PID ");
        crate::serial::put_hex32(pid);
        crate::serial::puts("\n");

        code as u64
    } else {
        u64::MAX - 10
    }
}

unsafe fn free_process_resources(proc: &mut Process) {
    if proc.kernel_stack_base != 0 && is_registry_initialized() {
        let pages = (PROCESS_KERNEL_STACK_SIZE as u64).div_ceil(PAGE_SIZE);
        let mut registry = global_registry_mut();
        let _ = registry.free_pages(proc.kernel_stack_base, pages);
        proc.kernel_stack_base = 0;
        proc.kernel_stack_top = 0;
    }

    if proc.fb_surface_phys != 0 && proc.fb_surface_pages != 0 && is_registry_initialized() {
        let mut registry = global_registry_mut();
        let _ = registry.free_pages(proc.fb_surface_phys, proc.fb_surface_pages);
        proc.fb_surface_phys = 0;
        proc.fb_surface_pages = 0;
        proc.fb_surface_dirty = false;
    }

    if is_registry_initialized() {
        let mut registry = global_registry_mut();
        for (_, vma) in proc.vma_table.iter() {
            if vma.owns_phys {
                let _ = registry.free_pages(vma.phys, vma.pages);
            }
        }
    }

    if proc.cr3 != 0 && proc.pid != 0 && proc.thread_group_leader == 0 {
        let kernel_cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) kernel_cr3, options(nostack, nomem));
        let kernel_cr3 = kernel_cr3 & 0x000F_FFFF_FFFF_F000;

        if proc.cr3 != kernel_cr3 {
            free_user_page_tables(proc.cr3);
            proc.cr3 = 0;
        }
    }
}

unsafe fn free_user_page_tables(pml4_phys: u64) {
    if !is_registry_initialized() {
        return;
    }
    let mut registry = global_registry_mut();

    let pml4 = pml4_phys as *const u64;

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
                let _ = registry.free_pages(pt_phys, 1);
            }
            let _ = registry.free_pages(pd_phys, 1);
        }
        let _ = registry.free_pages(pdpt_phys, 1);
    }
    let _ = registry.free_pages(pml4_phys, 1);
}
