// Process/system info syscalls: clock/sysinfo/getppid/spawn.

extern crate alloc;

use super::common::*;
use crate::hal;
use crate::process::ProcessState;
use crate::schedular::{PROCESS_TABLE, PROCESS_TABLE_LOCK, SCHEDULER};
use alloc::vec::Vec;
use morpheus_foundation::errno::E2BIG;
use morpheus_foundation::flags::open_flags::{O_PIPE_READ, O_PIPE_WRITE};
use morpheus_foundation::flags::{
    SPAWN_CLEAR_FDS, SPAWN_FA_CHDIR, SPAWN_FA_CLOSE, SPAWN_FA_DUP2, SPAWN_FA_OPEN,
};
use morpheus_foundation::types::{SpawnArgs, SpawnFileAction};
use morpheus_foundation::PAGE_SIZE;
use morpheus_hal_api::{AllocKind, MemoryType};

use morpheus_foundation::types::{SysInfo, SYSINFO_MAX_CPUS};

const MAX_ENV_BYTES: usize = 16 * 1024;
const MAX_FILE_ACTIONS: u64 = 64;

/// Monotonic ns from TSC. 0 if TSC isn't calibrated. Shares the one kernel time
/// source with SYS_CLOCK_GETTIME so the scalar and Timespec paths can't drift.
pub unsafe fn sys_clock() -> u64 {
    crate::clock::monotonic_ns()
}

/// SYS_SET_THREAD_POINTER: set the calling thread's TLS base (x86 FS base).
/// Validates the canonical-user range (which also makes the `wrmsr` safe),
/// records it on the Process for per-switch restore, and applies it live for
/// the current thread. `tp == 0` clears TLS.
pub unsafe fn sys_set_thread_pointer(tp: u64) -> u64 {
    // Canonical lower-half: both the user/kernel boundary and wrmsr-#GP safety
    // (AMD64 Vol 2 §5.3). 0 is allowed and falls inside the range.
    if tp >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    SCHEDULER.current_process_mut().tls_base = tp;
    hal().cpu().set_user_tls_base(tp);
    0
}

pub unsafe fn sys_sysinfo(buf_ptr: u64) -> u64 {
    let size = core::mem::size_of::<SysInfo>() as u64;
    if !validate_user_buf(buf_ptr, size) {
        return EFAULT;
    }

    let phys = hal().phys();
    let total_mem = phys.total_memory();
    let free_mem = phys.free_memory();
    let used = phys.allocated_memory();

    let tsc_freq = crate::schedular::tsc_frequency();

    let cpu_count = hal().smp().cpu_count();
    let mut per_core_idle_tsc = [0u64; SYSINFO_MAX_CPUS];
    crate::schedular::sample_per_core_idle_tsc(&mut per_core_idle_tsc);

    let info = SysInfo {
        total_mem,
        free_mem,
        num_procs: SCHEDULER.live_count(),
        cpu_count,
        uptime_ticks: hal().timer().read_tsc(),
        tsc_freq,
        heap_total: total_mem,
        heap_used: used,
        heap_free: free_mem,
        sched_ticks: SCHEDULER.tick_count() as u64,
        idle_tsc: crate::schedular::idle_tsc_total(),
        per_core_idle_tsc,
        ..SysInfo::zeroed()
    };

    let dst = buf_ptr as *mut SysInfo;
    core::ptr::write(dst, info);
    0
}

pub unsafe fn sys_getppid() -> u64 {
    let proc = SCHEDULER.current_process_mut();
    proc.parent_pid as u64
}

/// Flatten an array of `argc` `{ptr,len}` pairs at `argv_ptr` into a
/// NUL-separated blob (≤256 B, ≤16 entries, each ≤127 B) for `Process.args`.
/// Returns `(blob_len, count)`.
unsafe fn build_arg_blob(
    argv_ptr: u64,
    argc: u64,
    out: &mut [u8; 256],
) -> Result<(usize, u8), u64> {
    let mut blob_len = 0usize;
    let mut count = 0u8;
    if argc == 0 || argv_ptr == 0 {
        return Ok((0, 0));
    }
    if argc > 16 {
        return Err(E2BIG);
    }
    if !validate_user_buf(argv_ptr, argc.saturating_mul(16)) {
        return Err(EFAULT);
    }
    let argv = core::slice::from_raw_parts(argv_ptr as *const [u64; 2], argc as usize);
    for pair in argv {
        let (a_ptr, a_len) = (pair[0], pair[1] as usize);
        if a_ptr == 0 || a_len == 0 || a_len > 127 {
            continue;
        }
        if !validate_user_buf(a_ptr, a_len as u64) {
            return Err(EFAULT);
        }
        if blob_len + a_len + 1 > 256 {
            break;
        }
        let src = core::slice::from_raw_parts(a_ptr as *const u8, a_len);
        out[blob_len..blob_len + a_len].copy_from_slice(src);
        blob_len += a_len;
        out[blob_len] = 0;
        blob_len += 1;
        count += 1;
    }
    Ok((blob_len, count))
}

/// Flatten an array of `envc` `{ptr,len}` `KEY=VALUE` pairs into a heap
/// NUL-separated environ block (≤`MAX_ENV_BYTES`). Backs SYS_GETENV / std::env.
unsafe fn build_env_blob(envp_ptr: u64, envc: u64) -> Result<Vec<u8>, u64> {
    let mut blob = Vec::new();
    if envc == 0 || envp_ptr == 0 {
        return Ok(blob);
    }
    if envc > 1024 {
        return Err(E2BIG);
    }
    if !validate_user_buf(envp_ptr, envc.saturating_mul(16)) {
        return Err(EFAULT);
    }
    let envp = core::slice::from_raw_parts(envp_ptr as *const [u64; 2], envc as usize);
    for pair in envp {
        let (e_ptr, e_len) = (pair[0], pair[1] as usize);
        if e_ptr == 0 || e_len == 0 {
            continue;
        }
        if !validate_user_buf(e_ptr, e_len as u64) {
            return Err(EFAULT);
        }
        if blob.len() + e_len + 1 > MAX_ENV_BYTES {
            return Err(E2BIG);
        }
        let src = core::slice::from_raw_parts(e_ptr as *const u8, e_len);
        blob.extend_from_slice(src);
        blob.push(0);
    }
    Ok(blob)
}

/// Replay `file_actions[]` in array order over the spawned child's fd table. Held
/// under PROCESS_TABLE_LOCK throughout, so the just-Ready child cannot be
/// scheduled on another core mid-replay. `fa_stride` (not our `size_of`) is the
/// indexing stride, for forward-compatible record growth.
unsafe fn replay_file_actions(child_pid: u32, sa: &SpawnArgs) -> Result<(), u64> {
    let count = sa.file_actions_count;
    if count == 0 || sa.file_actions_ptr == 0 {
        return Ok(());
    }
    if count > MAX_FILE_ACTIONS {
        return Err(E2BIG);
    }
    let stride = sa.fa_stride as u64;
    if (stride as usize) < core::mem::size_of::<SpawnFileAction>() {
        return Err(EINVAL);
    }
    let total = match stride.checked_mul(count) {
        Some(t) => t,
        None => return Err(EINVAL),
    };
    if !validate_user_buf(sa.file_actions_ptr, total) {
        return Err(EFAULT);
    }

    PROCESS_TABLE_LOCK.lock();
    let mut result = Ok(());
    for i in 0..count {
        let fa = core::ptr::read((sa.file_actions_ptr + i * stride) as *const SpawnFileAction);
        result = match fa.op {
            SPAWN_FA_CLOSE => child_fd_close(child_pid, fa.fd),
            SPAWN_FA_DUP2 => child_fd_dup2(child_pid, fa.fd, fa.newfd),
            SPAWN_FA_CHDIR => child_chdir(child_pid, fa.path_ptr, fa.path_len),
            SPAWN_FA_OPEN => child_fd_open(child_pid, fa.fd, fa.path_ptr, fa.path_len, fa.oflags),
            _ => Err(EINVAL),
        };
        if result.is_err() {
            break;
        }
    }
    PROCESS_TABLE_LOCK.unlock();
    result
}

/// PROCESS_TABLE_LOCK held by caller.
unsafe fn child_fd_close(child_pid: u32, fd: i32) -> Result<(), u64> {
    if fd < 0 {
        return Err(EBADF);
    }
    let child = match PROCESS_TABLE
        .get_mut(child_pid as usize)
        .and_then(|s| s.as_mut())
    {
        Some(c) => c,
        None => return Err(ESRCH),
    };
    let desc = match child.fd_table.get(fd as usize) {
        Some(d) => *d,
        None => return Ok(()), // closing an unopened fd is a no-op in posix_spawn
    };
    if desc.flags & (O_PIPE_READ | O_PIPE_WRITE) != 0 {
        let idx = desc.mount_id as u8;
        child.fd_table.free(fd as usize);
        if desc.flags & O_PIPE_READ != 0 {
            crate::pipe::pipe_close_reader(idx);
        }
        if desc.flags & O_PIPE_WRITE != 0 {
            crate::pipe::pipe_close_writer(idx);
        }
        return Ok(());
    }
    // File fd: close through storage to drop the per-mount refcount.
    {
        let guard = crate::storage::lock();
        let g = &mut *guard.g;
        if let Some((m, dev)) = g.mount_dev_mut(desc.mount_id) {
            let _ = m.fs.close(dev, &desc);
            m.open_fds = m.open_fds.saturating_sub(1);
        }
    }
    child.fd_table.free(fd as usize);
    Ok(())
}

/// PROCESS_TABLE_LOCK held by caller. dup2 within the child fd table.
unsafe fn child_fd_dup2(child_pid: u32, old_fd: i32, new_fd: i32) -> Result<(), u64> {
    if old_fd < 0 || new_fd < 0 {
        return Err(EBADF);
    }
    if old_fd == new_fd {
        let child = PROCESS_TABLE
            .get(child_pid as usize)
            .and_then(|s| s.as_ref())
            .ok_or(ESRCH)?;
        return if child.fd_table.get(old_fd as usize).is_some() {
            Ok(())
        } else {
            Err(EBADF)
        };
    }
    // Close any existing new_fd first (POSIX dup2 semantics).
    let _ = child_fd_close(child_pid, new_fd);

    let child = match PROCESS_TABLE
        .get_mut(child_pid as usize)
        .and_then(|s| s.as_mut())
    {
        Some(c) => c,
        None => return Err(ESRCH),
    };
    let src = match child.fd_table.get(old_fd as usize) {
        Some(d) => *d,
        None => return Err(EBADF),
    };
    if !child.fd_table.set(new_fd as usize, src) {
        return Err(EBADF);
    }
    let idx = src.mount_id as u8;
    if src.flags & O_PIPE_READ != 0 {
        crate::pipe::pipe_add_reader(idx);
    }
    if src.flags & O_PIPE_WRITE != 0 {
        crate::pipe::pipe_add_writer(idx);
    }
    if src.flags & (O_PIPE_READ | O_PIPE_WRITE) == 0 {
        let guard = crate::storage::lock();
        let g = &mut *guard.g;
        if let Some((m, _)) = g.mount_dev_mut(src.mount_id) {
            m.open_fds = m.open_fds.saturating_add(1);
        }
    }
    Ok(())
}

/// PROCESS_TABLE_LOCK held by caller. Set the child's cwd.
unsafe fn child_chdir(child_pid: u32, path_ptr: u64, path_len: u64) -> Result<(), u64> {
    let path = user_path(path_ptr, path_len).ok_or(EINVAL)?;
    let child = PROCESS_TABLE
        .get_mut(child_pid as usize)
        .and_then(|s| s.as_mut())
        .ok_or(ESRCH)?;
    child.set_cwd(path);
    Ok(())
}

/// PROCESS_TABLE_LOCK held by caller. Open `path` into the child fd table at the
/// exact `fd` (posix_spawn_file_actions_addopen semantics).
unsafe fn child_fd_open(
    child_pid: u32,
    fd: i32,
    path_ptr: u64,
    path_len: u64,
    oflags: u32,
) -> Result<(), u64> {
    if fd < 0 || fd as usize >= crate::storage::fs_api::FD_TABLE_LEN {
        return Err(EBADF);
    }
    let path = user_path(path_ptr, path_len).ok_or(EINVAL)?;
    let ts = hal().timer().read_tsc();

    let guard = crate::storage::lock();
    let g = &mut *guard.g;
    let (mount_id, m, dev, rel) = g.resolve_mut(path).ok_or(ENOENT)?;
    let opened = match m.fs.open(dev, rel, oflags, ts) {
        Ok(o) => o,
        Err(e) => return Err(crate::storage::vfs_err_to_errno(e)),
    };
    let mut state = crate::storage::fs_api::FdState::empty();
    state.mount_id = mount_id;
    state.flags = oflags;
    let pb = rel.as_bytes();
    let n = pb.len().min(state.path.len());
    state.path[..n].copy_from_slice(&pb[..n]);
    state.path_len = n as u16;
    state.cookie = opened.cookie;
    m.open_fds = m.open_fds.saturating_add(1);
    drop(guard);

    let _ = child_fd_close(child_pid, fd);
    let child = PROCESS_TABLE
        .get_mut(child_pid as usize)
        .and_then(|s| s.as_mut())
        .ok_or(ESRCH)?;
    if !child.fd_table.set(fd as usize, state) {
        return Err(EBADF);
    }
    Ok(())
}

/// SYS_SPAWN(*const SpawnArgs) — posix_spawn. The child inherits the parent fd
/// table minus `O_CLOEXEC` (or empty if `SPAWN_CLEAR_FDS`), then `file_actions[]`
/// replay in order; argv/envp/cwd come off the versioned block.
pub unsafe fn sys_spawn(args_ptr: u64) -> u64 {
    if args_ptr == 0
        || args_ptr & 7 != 0
        || !validate_user_buf(args_ptr, core::mem::size_of::<SpawnArgs>() as u64)
    {
        return EFAULT;
    }
    let sa = core::ptr::read(args_ptr as *const SpawnArgs);

    let path = match user_path(sa.path_ptr, sa.path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    let mut arg_blob = [0u8; 256];
    let (blob_len, arg_count) = match build_arg_blob(sa.argv_ptr, sa.argc, &mut arg_blob) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let env_blob = match build_env_blob(sa.envp_ptr, sa.envc) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let cwd = if sa.cwd_ptr != 0 {
        match user_path(sa.cwd_ptr, sa.cwd_len) {
            Some(p) => Some(p),
            None => return EINVAL,
        }
    } else {
        None
    };
    let clear_fds = sa.flags & SPAWN_CLEAR_FDS != 0;

    let ts = hal().timer().read_tsc();

    let file_size = {
        let guard = crate::storage::lock();
        let g = &mut *guard.g;
        let (_, m, dev, rel) = match g.resolve_mut(path) {
            Some(t) => t,
            None => return ENOENT,
        };
        match m.fs.stat(dev, rel) {
            Ok(s) => s.size as usize,
            // Translate the VFS error so a missing program yields ENOENT (→ std
            // ErrorKind::NotFound), not a blanket EIO that surfaces as Uncategorized.
            Err(e) => return crate::storage::vfs_err_to_errno(e),
        }
    };

    if file_size == 0 || file_size > 4 * 1024 * 1024 {
        return EINVAL;
    }

    let pages_needed = file_size.div_ceil(PAGE_SIZE as usize) as u64;
    let buf_phys =
        match hal()
            .phys()
            .allocate_pages(AllocKind::AnyPages, MemoryType::Allocated, pages_needed)
        {
            Ok(addr) => addr,
            Err(_) => return ENOMEM,
        };

    // Read the image under the lock with a transient FdState, then drop the
    // guard before spawn_user_process — load_elf64 reacquires the registry.
    let buf = core::slice::from_raw_parts_mut(buf_phys as *mut u8, file_size);
    let bytes_read = {
        let guard = crate::storage::lock();
        let g = &mut *guard.g;
        let (mount_id, m, dev, rel) = match g.resolve_mut(path) {
            Some(t) => t,
            None => {
                let _ = hal().phys().free_pages(buf_phys, pages_needed);
                return ENOENT;
            },
        };
        let opened = match m.fs.open(dev, rel, 0x01, ts) {
            Ok(o) => o,
            Err(_) => {
                let _ = hal().phys().free_pages(buf_phys, pages_needed);
                return ENOENT;
            },
        };
        let mut fdstate = crate::storage::fs_api::FdState::empty();
        fdstate.mount_id = mount_id;
        fdstate.cookie = opened.cookie;
        // Backend reads key off the (mount-relative) path, not just the cookie.
        let pb = rel.as_bytes();
        let pn = pb.len().min(fdstate.path.len());
        fdstate.path[..pn].copy_from_slice(&pb[..pn]);
        fdstate.path_len = pn as u16;
        let n = match m.fs.read(dev, &fdstate, buf) {
            Ok(n) => n,
            Err(_) => {
                let _ = m.fs.close(dev, &fdstate);
                let _ = hal().phys().free_pages(buf_phys, pages_needed);
                return EIO;
            },
        };
        let _ = m.fs.close(dev, &fdstate);
        n
    };

    let name = path.rsplit('/').next().unwrap_or(path);

    let elf_data = &buf[..bytes_read];
    let result = crate::schedular::spawn_user_process(
        name,
        elf_data,
        &arg_blob[..blob_len],
        arg_count,
        &env_blob,
        cwd,
        true,
        clear_fds,
    );

    let _ = hal().phys().free_pages(buf_phys, pages_needed);

    match result {
        Ok(pid) => pid as u64,
        Err(_) => ENOMEM,
    }
}

/// `SYS_REPARENT(target_pid, new_parent_pid) -> 0 | -errno`. Re-points a
/// process's `parent_pid`. A generic process-tree primitive: the kernel has no
/// notion of init/userland, only the parent/child link it already tracks.
///
/// Permission (policy-free mechanism):
///  - **hand-off** — the caller is the target's current parent (give your child
///    away, e.g. a supervisor spawning then handing the child to its real owner), or
///  - **adopt-orphan** — the caller *is* `new_parent` and the target's current
///    parent is dead (Zombie/Terminated/absent), e.g. a respawned supervisor
///    adopting the orphaned tree. You may never steal a live process's child.
pub unsafe fn sys_reparent(target_pid: u64, new_parent_pid: u64) -> u64 {
    // pid 0 is the kernel/idle context — never a valid reparent participant.
    if target_pid == 0 || new_parent_pid == 0 {
        return EINVAL;
    }
    let target = target_pid as usize;
    let new_parent = new_parent_pid as usize;
    let caller = SCHEDULER.current_pid() as usize;

    PROCESS_TABLE_LOCK.lock();

    // The new parent must exist and be live — no adopting onto a dead/free slot.
    let new_parent_live = matches!(
        PROCESS_TABLE.get(new_parent).and_then(|s| s.as_ref()),
        Some(p) if !p.is_free()
            && !matches!(p.state, ProcessState::Zombie | ProcessState::Terminated)
    );
    if !new_parent_live {
        PROCESS_TABLE_LOCK.unlock();
        return ESRCH;
    }

    // The target must exist; read its current parent to decide permission.
    let cur_parent = match PROCESS_TABLE.get(target).and_then(|s| s.as_ref()) {
        Some(p) if !p.is_free() => p.parent_pid as usize,
        _ => {
            PROCESS_TABLE_LOCK.unlock();
            return ESRCH;
        },
    };

    let handoff = caller == cur_parent;
    let cur_parent_live = matches!(
        PROCESS_TABLE.get(cur_parent).and_then(|s| s.as_ref()),
        Some(p) if !p.is_free()
            && !matches!(p.state, ProcessState::Zombie | ProcessState::Terminated)
    );
    let adopt_orphan = caller == new_parent && !cur_parent_live;
    if !handoff && !adopt_orphan {
        PROCESS_TABLE_LOCK.unlock();
        return EPERM;
    }

    if let Some(Some(t)) = PROCESS_TABLE.get_mut(target) {
        t.parent_pid = new_parent as u32;
    }
    PROCESS_TABLE_LOCK.unlock();
    0
}
