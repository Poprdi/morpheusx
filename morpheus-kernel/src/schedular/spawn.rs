use super::lifecycle::apply_default_scheduler_policy;
use super::state::{this_core_pid, LIVE_COUNT, PROCESS_TABLE, PROCESS_TABLE_LOCK, SCHEDULER_READY};
use crate::hal;
use crate::process::{CpuContext, Process, ProcessState, MAX_PROCESSES, MAX_USER_PROCESSES};
use core::sync::atomic::Ordering;
use morpheus_foundation::flags::open_flags;
use morpheus_foundation::flags::THREAD_DETACHED;

const PAGE_SIZE: u64 = 4096;
/// Stack top for the user process. Mirrors `elf::USER_STACK_TOP`.
const USER_STACK_TOP: u64 = 0x0000_007F_FFFF_F000;

/// Live independent processes (thread-group leaders, PID != 0). Threads are
/// excluded — they draw from the separate thread budget, not this cap.
unsafe fn count_user_processes() -> usize {
    let mut n = 0;
    for slot in PROCESS_TABLE.iter().flatten() {
        if !slot.is_free() && slot.pid != 0 && !slot.is_thread() {
            n += 1;
        }
    }
    n
}

/// Lowest free slot at/above 1, reclaiming detached-thread zombies in passing.
unsafe fn find_free_slot() -> Option<usize> {
    super::wait::reap_detached_zombies();
    (1..MAX_PROCESSES).find(|&i| {
        PROCESS_TABLE[i]
            .as_ref()
            .map(|p| p.is_free())
            .unwrap_or(true)
    })
}

/// Thread sharing the caller's CR3. TLS base is set at creation (no
/// SET_THREAD_POINTER race); `ctid_ptr` drives CLONE_CHILD_CLEARTID for race-free
/// join. Draws from the full table, NOT the user-process cap.
pub unsafe fn spawn_user_thread(
    entry: u64,
    stack_top: u64,
    arg: u64,
    tls_base: u64,
    ctid_ptr: u64,
    flags: u64,
) -> Result<u32, &'static str> {
    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    PROCESS_TABLE_LOCK.lock();

    let parent_pid = this_core_pid();
    let parent = match PROCESS_TABLE
        .get(parent_pid as usize)
        .and_then(|s| s.as_ref())
    {
        Some(p) => p,
        None => {
            PROCESS_TABLE_LOCK.unlock();
            return Err("no current process");
        },
    };
    let parent_cr3 = parent.cr3;
    let parent_mmap_brk = parent.mmap_brk;
    let parent_cwd = parent.cwd;
    let parent_cwd_len = parent.cwd_len;

    let group_leader = if parent.thread_group_leader != 0 {
        parent.thread_group_leader
    } else {
        parent_pid
    };

    let slot_idx = match find_free_slot() {
        Some(i) => i,
        None => {
            PROCESS_TABLE_LOCK.unlock();
            return Err("process table full");
        },
    };

    let tid = slot_idx as u32;
    // Fresh occupant of a reused slot starts with blocking stdin.
    crate::process::set_stdin_nonblock(tid, false);

    PROCESS_TABLE[slot_idx] = Some(Process::empty());
    let thread = PROCESS_TABLE[slot_idx].as_mut().ok_or_else(|| {
        PROCESS_TABLE_LOCK.unlock();
        "failed to initialize thread slot"
    })?;

    thread.pid = tid;
    thread.set_name("thread");
    thread.parent_pid = parent_pid;
    thread.priority = 128;
    thread.state = ProcessState::Ready;
    thread.cr3 = parent_cr3;
    thread.thread_group_leader = group_leader;
    thread.mmap_brk = parent_mmap_brk;
    thread.cwd = parent_cwd;
    thread.cwd_len = parent_cwd_len;
    thread.tls_base = tls_base;
    thread.ctid_ptr = ctid_ptr;
    thread.detached = flags & THREAD_DETACHED != 0;
    apply_default_scheduler_policy(thread, false);

    if let Some(Some(parent_ref)) = PROCESS_TABLE.get(parent_pid as usize) {
        thread.importance_16 = parent_ref.importance_16;
        thread.power_mode = parent_ref.power_mode;
        thread.policy_class = parent_ref.policy_class;
        thread.affinity_mask = parent_ref.affinity_mask;
        thread.policy_flags = parent_ref.policy_flags;
        thread.capability_bits = parent_ref.capability_bits;
    }

    if let Err(e) = thread.alloc_kernel_stack() {
        PROCESS_TABLE[slot_idx] = None;
        PROCESS_TABLE_LOCK.unlock();
        return Err(e);
    }

    {
        // `arg` lands in arg slot 0 (rdi on x86_64).
        thread.context = CpuContext::zeroed();
        hal().cpu().ctx_init_user(
            &mut thread.context,
            entry,
            stack_top - 8,
            &[arg, 0, 0, 0, 0, 0],
        );
    }

    crate::serial::puts("[SCHED] spawned TID ");
    crate::serial::put_hex32(tid);
    crate::serial::puts(" group=");
    crate::serial::put_hex32(group_leader);
    crate::serial::puts("\n");

    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);
    PROCESS_TABLE_LOCK.unlock();
    Ok(tid)
}

/// Inherit the parent's fds minus `O_CLOEXEC`. Inherited pipe endpoints bump the
/// per-pipe refcount so the fd keeps the pipe alive.
unsafe fn inherit_fds_minus_cloexec(child: &mut Process, parent: &Process) {
    let mut seen_readers: [bool; 256] = [false; 256];
    let mut seen_writers: [bool; 256] = [false; 256];
    for (fd, desc) in parent.fd_table.iter() {
        if desc.flags & open_flags::O_CLOEXEC != 0 {
            continue;
        }
        if !child.fd_table.set(fd, *desc) {
            continue;
        }
        let idx = desc.mount_id as u8 as usize;
        if desc.flags & open_flags::O_PIPE_READ != 0 && !seen_readers[idx] {
            crate::pipe::pipe_add_reader(idx as u8);
            seen_readers[idx] = true;
        }
        if desc.flags & open_flags::O_PIPE_WRITE != 0 && !seen_writers[idx] {
            crate::pipe::pipe_add_writer(idx as u8);
            seen_writers[idx] = true;
        }
    }
}

/// Spawn an independent process from an ELF image. `clear_fds` overrides
/// `inherit_fds`, starting the child with an empty fd table.
#[allow(clippy::too_many_arguments)]
pub unsafe fn spawn_user_process(
    name: &str,
    elf_data: &[u8],
    arg_blob: &[u8],
    arg_count: u8,
    env_block: &[u8],
    cwd: Option<&str>,
    inherit_fds: bool,
    clear_fds: bool,
) -> Result<u32, &'static str> {
    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    let image = crate::elf::load_elf64(elf_data).map_err(|_| "ELF load failed")?;

    PROCESS_TABLE_LOCK.lock();

    if count_user_processes() >= MAX_USER_PROCESSES {
        PROCESS_TABLE_LOCK.unlock();
        return Err("user process cap reached");
    }

    let slot_idx = match find_free_slot() {
        Some(i) => i,
        None => {
            PROCESS_TABLE_LOCK.unlock();
            return Err("process table full");
        },
    };

    let pid = slot_idx as u32;
    // Fresh occupant of a reused slot starts with blocking stdin.
    crate::process::set_stdin_nonblock(pid, false);

    let mut proc = Process::empty();
    proc.pid = pid;
    proc.set_name(name);
    proc.parent_pid = this_core_pid();
    proc.priority = 128;
    proc.state = ProcessState::Ready;
    proc.cr3 = image.pml4_phys;
    apply_default_scheduler_policy(&mut proc, false);

    if let Err(e) = proc.alloc_kernel_stack() {
        PROCESS_TABLE_LOCK.unlock();
        return Err(e);
    }

    // cwd: explicit override wins, else inherit the parent's.
    match cwd {
        Some(p) => proc.set_cwd(p),
        None => {
            if let Some(Some(parent)) = PROCESS_TABLE.get(proc.parent_pid as usize) {
                proc.cwd = parent.cwd;
                proc.cwd_len = parent.cwd_len;
            }
        },
    }

    if inherit_fds && !clear_fds {
        let parent_pid = proc.parent_pid as usize;
        if let Some(Some(parent)) = PROCESS_TABLE.get(parent_pid) {
            // SAFETY: parent and proc are distinct slots; copy out a raw ptr to
            // sidestep the borrow on the shared static table.
            let parent_ref: &Process = &*(parent as *const Process);
            inherit_fds_minus_cloexec(&mut proc, parent_ref);
        }
    }

    if !arg_blob.is_empty() && arg_count > 0 {
        let len = arg_blob.len().min(256);
        proc.args[..len].copy_from_slice(&arg_blob[..len]);
        proc.args_len = len as u16;
        proc.argc = arg_count;
    }

    if !env_block.is_empty() {
        proc.env_block.clear();
        proc.env_block.extend_from_slice(env_block);
    }

    {
        // No args here: libmorpheus's _start reads them via SYS_GETARGS.
        proc.context = CpuContext::zeroed();
        hal().cpu().ctx_init_user(
            &mut proc.context,
            image.entry,
            USER_STACK_TOP - 8,
            &[0, 0, 0, 0, 0, 0],
        );
    }

    for &(vaddr, phys, memsz) in &image.segments {
        let pages = memsz / PAGE_SIZE;
        let _ = proc.vma_table.insert(vaddr, phys, pages, true);
    }

    let total_pages: u64 = image.segments.iter().map(|s| s.2 / 4096).sum();
    proc.pages_allocated = total_pages;

    let _ = (pid, image.entry, proc.cr3);
    crate::serial::log_info("SCHED", 771, "user process spawned");

    PROCESS_TABLE[slot_idx] = Some(proc);
    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);

    PROCESS_TABLE_LOCK.unlock();
    Ok(pid)
}
