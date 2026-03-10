use super::state::{this_core_pid, LIVE_COUNT, PROCESS_TABLE, PROCESS_TABLE_LOCK, SCHEDULER_READY};
use super::lifecycle::apply_default_scheduler_policy;
use crate::memory::PAGE_SIZE;
use crate::process::{CpuContext, Process, ProcessState, MAX_PROCESSES};
use core::sync::atomic::Ordering;

pub unsafe fn spawn_user_thread(entry: u64, stack_top: u64, arg: u64) -> Result<u32, &'static str> {
    use crate::cpu::gdt::{USER_CS, USER_DS};

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
        }
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

    let slot_idx = (1..MAX_PROCESSES)
        .find(|&i| {
            PROCESS_TABLE[i]
                .as_ref()
                .map(|p| p.is_free())
                .unwrap_or(true)
        })
        .ok_or_else(|| {
            PROCESS_TABLE_LOCK.unlock();
            "process table full"
        })?;

    let tid = slot_idx as u32;

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
    apply_default_scheduler_policy(thread, false);

    // inherit parent policy for thread groups to keep scheduling intent coherent.
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

    thread.context = CpuContext {
        rip: entry,
        rsp: stack_top - 8,
        rdi: arg,
        rflags: 0x202,
        cs: USER_CS as u64,
        ss: USER_DS as u64,
        ..CpuContext::empty()
    };

    crate::serial::puts("[SCHED] spawned TID ");
    crate::serial::put_hex32(tid);
    crate::serial::puts(" group=");
    crate::serial::put_hex32(group_leader);
    crate::serial::puts("\n");

    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);
    PROCESS_TABLE_LOCK.unlock();
    Ok(tid)
}

pub unsafe fn spawn_user_process(
    name: &str,
    elf_data: &[u8],
    arg_blob: &[u8],
    arg_count: u8,
    inherit_fds: bool,
) -> Result<u32, &'static str> {
    use crate::cpu::gdt::{USER_CS, USER_DS};
    use crate::elf::{load_elf64, USER_STACK_TOP};

    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    let (image, page_table) = load_elf64(elf_data).map_err(|e| {
        use crate::elf::ElfError;
        use crate::serial::puts;
        puts("[SCHED] ELF load error: ");
        match e {
            ElfError::TooSmall => puts("too small\n"),
            ElfError::BadMagic => puts("bad magic\n"),
            ElfError::Not64Bit => puts("not 64-bit\n"),
            ElfError::NotLittleEndian => puts("not little-endian\n"),
            ElfError::NotX86_64 => puts("not x86-64\n"),
            ElfError::NotExecutable => puts("not executable (e_type)\n"),
            ElfError::BadPhdr => puts("bad program header\n"),
            ElfError::NoLoadSegments => puts("no PT_LOAD segments\n"),
            ElfError::MapFailed => puts("page mapping failed\n"),
            ElfError::AllocFailed => puts("physical page alloc failed\n"),
        }
        "ELF load failed"
    })?;

    PROCESS_TABLE_LOCK.lock();

    let slot_idx = (1..MAX_PROCESSES)
        .find(|&i| {
            PROCESS_TABLE[i]
                .as_ref()
                .map(|p| p.is_free())
                .unwrap_or(true)
        })
        .ok_or_else(|| {
            PROCESS_TABLE_LOCK.unlock();
            "process table full"
        })?;

    let pid = slot_idx as u32;

    let mut proc = Process::empty();
    proc.pid = pid;
    proc.set_name(name);
    proc.parent_pid = this_core_pid();
    proc.priority = 128;
    proc.state = ProcessState::Ready;
    proc.cr3 = page_table.pml4_phys;
    apply_default_scheduler_policy(&mut proc, false);

    if let Err(e) = proc.alloc_kernel_stack() {
        PROCESS_TABLE_LOCK.unlock();
        return Err(e);
    }

    if inherit_fds {
        let parent_pid = proc.parent_pid as usize;
        if let Some(Some(parent)) = PROCESS_TABLE.get(parent_pid) {
            proc.fd_table = parent.fd_table;
            use morpheus_helix::types::open_flags;
            let mut seen_readers: [bool; 256] = [false; 256];
            let mut seen_writers: [bool; 256] = [false; 256];
            for fd_desc in proc.fd_table.fds.iter() {
                if fd_desc.is_open() {
                    let fl = fd_desc.flags;
                    let idx = fd_desc.mount_idx as usize;
                    if idx < 256 {
                        if fl & open_flags::O_PIPE_READ != 0 && !seen_readers[idx] {
                            crate::pipe::pipe_add_reader(fd_desc.mount_idx);
                            seen_readers[idx] = true;
                        }
                        if fl & open_flags::O_PIPE_WRITE != 0 && !seen_writers[idx] {
                            crate::pipe::pipe_add_writer(fd_desc.mount_idx);
                            seen_writers[idx] = true;
                        }
                    }
                }
            }
        }
    }

    if !arg_blob.is_empty() && arg_count > 0 {
        let len = arg_blob.len().min(256);
        proc.args[..len].copy_from_slice(&arg_blob[..len]);
        proc.args_len = len as u16;
        proc.argc = arg_count;
    }

    proc.context = CpuContext {
        rip: image.entry,
        rsp: USER_STACK_TOP - 8,
        rflags: 0x202,
        cs: USER_CS as u64,
        ss: USER_DS as u64,
        ..CpuContext::empty()
    };

    for seg in &image.segments {
        let pages = seg.memsz / PAGE_SIZE;
        let _ = proc.vma_table.insert(seg.vaddr, seg.phys, pages, true);
    }

    let total_pages: u64 = image.segments.iter().map(|s| s.memsz / 4096).sum();
    proc.pages_allocated = total_pages;

    let _ = (pid, image.entry, proc.cr3);
    crate::serial::log_info("SCHED", 771, "user process spawned");

    PROCESS_TABLE[slot_idx] = Some(proc);
    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);

    PROCESS_TABLE_LOCK.unlock();
    Ok(pid)
}
