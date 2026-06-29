// VFS syscalls. All route through `crate::storage` (spec §5/§7):
// validate → STORAGE_LOCK → resolve → backend dispatch → VfsError→errno.

use super::common::*;
use crate::schedular::SCHEDULER;
use crate::storage::fs_api::FdKind;
use crate::storage::{self, vfs_err_to_errno};
use morpheus_foundation::errno::EXDEV;
use morpheus_foundation::flags::mode;
use morpheus_foundation::flags::open_flags::{
    O_APPEND, O_CLOEXEC, O_CREATE, O_EXCL, O_PIPE_READ, O_PIPE_WRITE, O_WRITE,
};
use morpheus_foundation::storage::{MNT_RDONLY, MNT_STAGED};
use morpheus_foundation::syscall_abi::{SEEK_CUR, SEEK_END, SEEK_SET};

pub unsafe fn sys_fs_open(path_ptr: u64, path_len: u64, flags: u64) -> u64 {
    let path = match resolve_user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let flags = flags as u32;
    let ts = fs_now_ns();
    let fd_table = SCHEDULER.current_fd_table_mut();

    // Fail fast on full fd table before touching the FS.
    let fd = match fd_table.alloc() {
        Some(fd) => fd,
        None => return EMFILE,
    };

    let guard = storage::lock();
    let g = &mut *guard.g;
    let (mount_id, m, dev, rel) = match g.resolve_mut(&path) {
        Some(t) => t,
        None => return ENOENT,
    };

    // O_EXCL (create_new): a real exists-check, not TOCTOU — we hold STORAGE_LOCK
    // across the probe and the create, so nothing can wedge the file in between.
    if flags & O_CREATE != 0 && flags & O_EXCL != 0 && m.fs.stat(dev, rel).is_ok() {
        return EEXIST;
    }

    let opened = match m.fs.open(dev, rel, flags, ts) {
        Ok(o) => o,
        Err(e) => return vfs_err_to_errno(e),
    };

    let mut state = crate::storage::fs_api::FdState::empty();
    state.mount_id = mount_id;
    state.flags = flags;
    state.kind = FdKind::Regular;
    state.cloexec = flags & O_CLOEXEC != 0;
    let pb = rel.as_bytes();
    let n = pb.len().min(state.path.len());
    state.path[..n].copy_from_slice(&pb[..n]);
    state.path_len = n as u16;
    state.cookie = opened.cookie;
    // O_APPEND: seed offset from backend stat now (lazy stat would need the engine).
    if flags & O_APPEND != 0 {
        if let Ok(st) = m.fs.stat(dev, rel) {
            state.offset = st.size;
        }
    }
    m.open_fds = m.open_fds.saturating_add(1);

    if !fd_table.set(fd, state) {
        // Slot vanished between alloc and set (shouldn't happen); undo refcount.
        if let Some((m, _)) = g.mount_dev_mut(mount_id) {
            m.open_fds = m.open_fds.saturating_sub(1);
        }
        return EMFILE;
    }
    fd as u64
}

pub unsafe fn sys_fs_close(fd: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();
    let desc = match fd_table.get(fd as usize) {
        Some(d) => *d,
        None => return EBADF,
    };

    // Socket fds carry a smoltcp backend handle + readiness slot to release.
    if desc.is_socket() {
        super::socket::socket_close_backend(&desc);
        return match fd_table.free(fd as usize) {
            Some(_) => 0,
            None => EBADF,
        };
    }

    if desc.flags & (O_PIPE_READ | O_PIPE_WRITE) != 0 {
        let idx = desc.mount_id as u8;
        if fd_table.free(fd as usize).is_none() {
            return EBADF;
        }
        // Drop the endpoint refcount so a peer sees EOF/EPIPE; closing the last
        // writer must wake any reader blocked for bytes that will never arrive.
        if desc.flags & O_PIPE_READ != 0 {
            crate::pipe::pipe_close_reader(idx);
        }
        if desc.flags & O_PIPE_WRITE != 0 {
            crate::pipe::pipe_close_writer(idx);
            crate::schedular::wake_pipe_readers(idx);
        }
        return 0;
    }

    // epoll fds have no mount/backend; closing one reclaims its watch set.
    if desc.kind == FdKind::Epoll {
        super::epoll::destroy_for(&desc);
        return match fd_table.free(fd as usize) {
            Some(_) => 0,
            None => EBADF,
        };
    }

    {
        let guard = storage::lock();
        let g = &mut *guard.g;
        if let Some((m, dev)) = g.mount_dev_mut(desc.mount_id) {
            let _ = m.fs.close(dev, &desc);
            m.open_fds = m.open_fds.saturating_sub(1);
        }
    }
    match fd_table.free(fd as usize) {
        Some(_) => 0,
        None => EBADF,
    }
}

pub unsafe fn sys_fs_read(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    if !validate_user_buf(buf_ptr, len) {
        return EFAULT;
    }
    let fd_table = SCHEDULER.current_fd_table_mut();
    let mut desc = match fd_table.get(fd as usize) {
        Some(d) => *d,
        None => return EBADF,
    };
    if desc.revoked {
        return EBADF;
    }
    if desc.is_socket() {
        return super::socket::socket_read(fd, buf_ptr, len);
    }
    // The authoritative cursor lives in the shared OFD for dup'd fds; seed the
    // copy the backend reads from it so aliased fds share one offset.
    desc.offset = fd_table.offset(fd as usize).unwrap_or(desc.offset);

    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize);
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (m, dev) = match g.mount_dev_mut(desc.mount_id) {
        Some(t) => t,
        None => return EBADF,
    };
    match m.fs.read(dev, &desc, buf) {
        Ok(n) => {
            fd_table.add_offset(fd as usize, n as u64);
            n as u64
        },
        Err(e) => vfs_err_to_errno(e),
    }
}

pub unsafe fn sys_fs_write(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    if !validate_user_buf(buf_ptr, len) {
        return EFAULT;
    }
    let fd_table = SCHEDULER.current_fd_table_mut();
    let mut desc = match fd_table.get(fd as usize) {
        Some(d) => *d,
        None => return EBADF,
    };
    if desc.revoked {
        return EBADF;
    }
    if desc.is_socket() {
        return super::socket::socket_write(fd, buf_ptr, len);
    }
    let status = fd_table.status_flags(fd as usize).unwrap_or(desc.flags);
    if status & O_WRITE == 0 {
        return EBADF;
    }
    desc.offset = fd_table.offset(fd as usize).unwrap_or(desc.offset);

    let buf = core::slice::from_raw_parts(buf_ptr as *const u8, len as usize);
    let ts = fs_now_ns();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (m, dev) = match g.mount_dev_mut(desc.mount_id) {
        Some(t) => t,
        None => return EBADF,
    };
    // O_APPEND atomically retargets the cursor to EOF before each write (POSIX);
    // the backend is whole-file under STORAGE_LOCK, so this is race-free.
    if status & O_APPEND != 0 {
        if let Ok(st) = m.fs.stat(dev, desc.path_str()) {
            desc.offset = st.size;
        }
    }
    match m.fs.write(dev, &mut desc, buf, ts) {
        Ok(n) => {
            fd_table.set_offset(fd as usize, desc.offset);
            n as u64
        },
        Err(e) => vfs_err_to_errno(e),
    }
}

pub unsafe fn sys_fs_seek(fd: u64, offset: u64, whence: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();
    let desc = match fd_table.get(fd as usize) {
        Some(d) => *d,
        None => return EBADF,
    };

    let base = match whence {
        SEEK_SET => 0i64,
        SEEK_CUR => fd_table.offset(fd as usize).unwrap_or(desc.offset) as i64,
        SEEK_END => {
            let guard = storage::lock();
            let g = &mut *guard.g;
            let (m, dev) = match g.mount_dev_mut(desc.mount_id) {
                Some(t) => t,
                None => return EBADF,
            };
            match m.fs.stat(dev, desc.path_str()) {
                Ok(st) => st.size as i64,
                Err(e) => return vfs_err_to_errno(e),
            }
        },
        _ => return EINVAL,
    };

    let new_off = base.saturating_add(offset as i64);
    if new_off < 0 {
        return EINVAL;
    }
    if fd_table.set_offset(fd as usize, new_off as u64) {
        new_off as u64
    } else {
        EBADF
    }
}

pub unsafe fn sys_fs_stat(path_ptr: u64, path_len: u64, stat_buf: u64) -> u64 {
    let path = match resolve_user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(&path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.stat(dev, rel) {
        Ok(mut stat) => {
            if stat_buf != 0 {
                let size = core::mem::size_of::<morpheus_foundation::types::FileStat>() as u64;
                if !validate_user_buf(stat_buf, size) {
                    return EFAULT;
                }
                fill_stat_metadata(&mut stat);
                *(stat_buf as *mut morpheus_foundation::types::FileStat) = stat;
            }
            0
        },
        Err(e) => vfs_err_to_errno(e),
    }
}

/// `readdir(path_ptr, path_len, buf_ptr, max_entries) → total_child_count`.
///
/// Writes `min(total, max_entries)` [`DirEntry`]s into `buf_ptr` and returns the
/// directory's *true* child count. `max_entries` is the caller-declared capacity of
/// `buf_ptr` in entries; the kernel never writes beyond it, so a too-small buffer is
/// truncated (and the `> max_entries` return signals the caller to retry larger) rather
/// than overrun. A null `buf_ptr` (or `max_entries == 0`) is a probe: nothing is written
/// and only the count is returned.
///
/// Without `max_entries` the kernel wrote its current child count regardless of the
/// caller's buffer size — a heap overflow whenever a directory held more children than
/// the caller allocated for. The capacity bound closes that at the syscall boundary;
/// userland keeps a grow-and-retry loop for directories larger than its initial guess.
pub unsafe fn sys_fs_readdir(path_ptr: u64, path_len: u64, buf_ptr: u64, max_entries: u64) -> u64 {
    let path = match resolve_user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(&path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.readdir(dev, rel) {
        Ok(entries) => {
            let total = entries.len();
            // Cap the write at the caller's declared capacity so the kernel can never
            // overrun the user buffer, however many children the directory holds.
            // `buf_ptr == 0` (or `max_entries == 0`) is a probe — write nothing.
            let writeable = if buf_ptr == 0 {
                0
            } else {
                core::cmp::min(total, max_entries as usize)
            };
            if writeable > 0 {
                let entry_size =
                    core::mem::size_of::<morpheus_foundation::types::DirEntry>() as u64;
                let span = (writeable as u64).saturating_mul(entry_size);
                // Validate only the span we actually write — a too-small buffer truncates,
                // it does not EFAULT.
                if !validate_user_buf(buf_ptr, span) {
                    return EFAULT;
                }
                let dst = buf_ptr as *mut morpheus_foundation::types::DirEntry;
                for (i, entry) in entries.iter().take(writeable).enumerate() {
                    *dst.add(i) = *entry;
                }
            }
            // Always return the TRUE total so a caller whose buffer was too small can
            // detect truncation (`total > max_entries`) and retry with a larger buffer.
            total as u64
        },
        Err(e) => vfs_err_to_errno(e),
    }
}

pub unsafe fn sys_fs_mkdir(path_ptr: u64, path_len: u64) -> u64 {
    let path = match resolve_user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let ts = fs_now_ns();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(&path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.mkdir(dev, rel, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

/// `SYS_UNLINK`: files only. A directory target → `EISDIR` (use `SYS_RMDIR`),
/// so `remove_file`/`remove_dir` stay type-correct. The type check and the
/// unlink share one `STORAGE_LOCK` critical section (no TOCTOU).
pub unsafe fn sys_fs_unlink(path_ptr: u64, path_len: u64) -> u64 {
    let path = match resolve_user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let ts = fs_now_ns();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(&path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.stat(dev, rel) {
        Ok(st) if st.mode & mode::S_IFMT == mode::S_IFDIR => return EISDIR,
        Ok(_) => {},
        Err(e) => return vfs_err_to_errno(e),
    }
    match m.fs.unlink(dev, rel, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

pub unsafe fn sys_fs_rename(old_ptr: u64, old_len: u64, new_ptr: u64, new_len: u64) -> u64 {
    let old = match resolve_user_path(old_ptr, old_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let new = match resolve_user_path(new_ptr, new_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let ts = fs_now_ns();
    let guard = storage::lock();
    let g = &mut *guard.g;

    // rename across mounts is EXDEV, never an implicit copy+delete (spec §4).
    let src_mount = match g.mounts.resolve(&old) {
        Some(id) => id,
        None => return ENOENT,
    };
    let dst_mount = match g.mounts.resolve(&new) {
        Some(id) => id,
        None => return ENOENT,
    };
    if src_mount != dst_mount {
        return EXDEV;
    }
    let mp_len = match g.mounts.get(src_mount) {
        Some(m) => m.mount_point_len as usize,
        None => return ENOENT,
    };
    let rel_old = storage::mount_relative(&old, mp_len);
    let rel_new = storage::mount_relative(&new, mp_len);
    let (m, dev) = match g.mount_dev_mut(src_mount) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.rename(dev, rel_old, rel_new, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

/// Shrink or zero-extend `path` to `new_size`.
pub unsafe fn sys_fs_truncate(path_ptr: u64, path_len: u64, new_size: u64) -> u64 {
    let path = match resolve_user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let ts = fs_now_ns();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(&path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.truncate(dev, rel, new_size, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

pub unsafe fn sys_fs_sync() -> u64 {
    let guard = storage::lock();
    let g = &mut *guard.g;
    let ids: alloc::vec::Vec<u64> = g.mounts.iter().map(|(id, _)| id).collect();
    for id in ids {
        if let Some((m, dev)) = g.mount_dev_mut(id) {
            let _ = m.fs.sync(dev);
        }
    }
    0
}

/// Named snapshot on the root mount; returns Helix LSN. Empty name = anonymous checkpoint.
pub unsafe fn sys_fs_snapshot(name_ptr: u64, name_len: u64) -> u64 {
    let name = if name_ptr == 0 || name_len == 0 {
        ""
    } else {
        match user_path(name_ptr, name_len) {
            Some(p) => p,
            None => return EINVAL,
        }
    };
    let ts = fs_now_ns();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, _rel) = match g.resolve_mut("/") {
        Some(t) => t,
        None => return ENODEV,
    };
    match m.fs.snapshot(dev, name, ts) {
        Ok(lsn) => lsn,
        Err(e) => vfs_err_to_errno(e),
    }
}

/// Fills `buf` with up to `max` `FileVersion` records (oldest first); `max == 0` probes count.
pub unsafe fn sys_fs_versions(path_ptr: u64, path_len: u64, buf_ptr: u64, max: u64) -> u64 {
    use morpheus_foundation::types::FileVersion;

    let path = match resolve_user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(&path) {
        Some(t) => t,
        None => return ENOENT,
    };
    let versions = match m.fs.versions(dev, rel) {
        Ok(v) => v,
        Err(e) => return vfs_err_to_errno(e),
    };

    if buf_ptr == 0 || max == 0 {
        return versions.len() as u64;
    }
    let n = versions.len().min(max as usize);
    if n == 0 {
        return 0;
    }
    let entry_size = core::mem::size_of::<FileVersion>() as u64;
    let total = (n as u64).saturating_mul(entry_size);
    if !validate_user_buf(buf_ptr, total) {
        return EFAULT;
    }
    let dst = buf_ptr as *mut FileVersion;
    for (i, (lsn, ts_ns, op)) in versions.iter().take(n).enumerate() {
        *dst.add(i) = FileVersion {
            lsn: *lsn,
            timestamp_ns: *ts_ns,
            op: *op,
            _pad: 0,
        };
    }
    n as u64
}

/// `SYS_VOLUMES`: fills `min(count, max)` `VolumeInfo` records; `max == 0` probes count.
pub unsafe fn sys_volumes(buf_ptr: u64, max: u64) -> u64 {
    use morpheus_block_types::DeviceKind;
    use morpheus_foundation::types::VolumeInfo;

    let guard = storage::lock();
    let g = &mut *guard.g;

    let count = g.volumes.iter().count();
    if buf_ptr == 0 || max == 0 {
        return count as u64;
    }
    let n = count.min(max as usize);
    if n == 0 {
        return 0;
    }
    let entry_size = core::mem::size_of::<VolumeInfo>() as u64;
    let total = (n as u64).saturating_mul(entry_size);
    if !validate_user_buf(buf_ptr, total) {
        return EFAULT;
    }

    let dst = buf_ptr as *mut VolumeInfo;
    for (i, (vol_id, v)) in g.volumes.iter().take(n).enumerate() {
        let device_kind = g
            .devices
            .get(v.device_id)
            .map(|d| d.kind.to_dev())
            .unwrap_or(DeviceKind::Ram.to_dev());
        let mut flags = 0u32;
        if v.read_only {
            flags |= morpheus_foundation::storage::VOL_RDONLY;
        }
        if v.mounted {
            flags |= morpheus_foundation::storage::VOL_MOUNTED;
        }
        if v.removable {
            flags |= morpheus_foundation::storage::VOL_REMOVABLE;
        }
        if v.ephemeral {
            flags |= morpheus_foundation::storage::VOL_EPHEMERAL;
        }
        *dst.add(i) = VolumeInfo {
            volume_id: vol_id,
            device_id: v.device_id,
            device_kind,
            fs_type: v.detected_fs,
            lba_start: v.lba_start,
            lba_count: v.lba_count,
            block_size: v.block_size,
            flags,
            partition_guid: v.partition_guid,
            label: v.label,
            ..VolumeInfo::zeroed()
        };
    }
    count as u64
}

/// `SYS_MOUNTS`: fills `min(count, max)` `MountInfo` records; `max == 0` probes count.
pub unsafe fn sys_mounts(buf_ptr: u64, max: u64) -> u64 {
    use morpheus_foundation::types::MountInfo;

    let guard = storage::lock();
    let g = &mut *guard.g;

    let count = g.mounts.iter().count();
    if buf_ptr == 0 || max == 0 {
        return count as u64;
    }
    let n = count.min(max as usize);
    if n == 0 {
        return 0;
    }
    let entry_size = core::mem::size_of::<MountInfo>() as u64;
    let total = (n as u64).saturating_mul(entry_size);
    if !validate_user_buf(buf_ptr, total) {
        return EFAULT;
    }

    let dst = buf_ptr as *mut MountInfo;
    for (i, (mount_id, m)) in g.mounts.iter().take(n).enumerate() {
        *dst.add(i) = MountInfo {
            mount_id,
            volume_id: m.volume_id,
            fs_type: m.fs_type,
            flags: m.flags,
            mount_point: m.mount_point,
            mount_point_len: m.mount_point_len,
            ..MountInfo::zeroed()
        };
    }
    count as u64
}

/// `SYS_MOUNT` (spec §5). `VOLUME_NONE` → fresh RAM; `MNT_STAGED` → copy-to-RAM.
/// `aux`: required size for RAM mounts, optional cap for staged. Returns `mount_id` or errno.
pub unsafe fn sys_mount(
    source_volume_id: u64,
    mp_ptr: u64,
    mp_len: u64,
    fs_type: u64,
    flags: u64,
    aux: u64,
) -> u64 {
    let mp = match user_path(mp_ptr, mp_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let mut mount_point = [0u8; 256];
    let pb = mp.as_bytes();
    let n = pb.len().min(256);
    mount_point[..n].copy_from_slice(&pb[..n]);

    // Userland mounts are unprivileged and charged to the caller's RAM budget (spec §6).
    let pid = SCHEDULER.current_process_mut().pid;

    let req = storage::MountReq {
        source_volume_id,
        mount_point,
        mount_point_len: n as u16,
        fs_type: fs_type as u32,
        flags: (flags as u32) & (MNT_RDONLY | MNT_STAGED),
        aux,
        pid,
        privileged: false,
    };
    match storage::mount(&req) {
        Ok(mount_id) => mount_id,
        Err(e) => e,
    }
}

/// `SYS_UMOUNT` (spec §5). Exact mountpoint match; `MNT_FORCE` revokes open fds.
pub unsafe fn sys_umount(mp_ptr: u64, mp_len: u64, flags: u64) -> u64 {
    let mp = match user_path(mp_ptr, mp_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    match storage::umount(mp, flags as u32) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// SYS_FSTAT: `fd,*mut FileStat -> 0 | -errno`. A non-regular fd (socket/pipe/
/// epoll) has no FS object, so its type/perm bits are synthesized (size 0) to keep
/// `fstat` well-defined on any fd.
pub unsafe fn sys_fs_fstat(fd: u64, statbuf: u64) -> u64 {
    use morpheus_foundation::types::FileStat;

    let size = core::mem::size_of::<FileStat>() as u64;
    if !validate_user_buf(statbuf, size) {
        return EFAULT;
    }
    let fd_table = SCHEDULER.current_fd_table_mut();
    let desc = match fd_table.get(fd as usize) {
        Some(d) => *d,
        None => return EBADF,
    };
    if desc.revoked {
        return EBADF;
    }

    if desc.kind != FdKind::Regular {
        let mut stat = FileStat::default();
        stat.mode = match desc.kind {
            FdKind::Socket => mode::S_IFSOCK,
            FdKind::Pipe => mode::S_IFIFO,
            FdKind::Epoll => mode::S_IFCHR,
            FdKind::Regular => mode::S_IFREG,
        };
        fill_stat_metadata(&mut stat);
        *(statbuf as *mut FileStat) = stat;
        return 0;
    }

    let guard = storage::lock();
    let g = &mut *guard.g;
    let (m, dev) = match g.mount_dev_mut(desc.mount_id) {
        Some(t) => t,
        None => return EBADF,
    };
    match m.fs.stat(dev, desc.path_str()) {
        Ok(mut stat) => {
            fill_stat_metadata(&mut stat);
            *(statbuf as *mut FileStat) = stat;
            0
        },
        Err(e) => vfs_err_to_errno(e),
    }
}

/// SYS_FSYNC: `fd,flags -> 0 | -errno`. Backends expose only whole-volume `sync`,
/// so fdatasync (`FSYNC_DATAONLY`) and full fsync share one durability point.
/// Non-regular fds have no durable backing, so succeed trivially.
pub unsafe fn sys_fs_fsync(fd: u64, _flags: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();
    let desc = match fd_table.get(fd as usize) {
        Some(d) => *d,
        None => return EBADF,
    };
    if desc.revoked {
        return EBADF;
    }
    if desc.kind != FdKind::Regular {
        return 0;
    }
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (m, dev) = match g.mount_dev_mut(desc.mount_id) {
        Some(t) => t,
        None => return EBADF,
    };
    match m.fs.sync(dev) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

/// SYS_FTRUNCATE: `fd,new_len -> 0 | -errno`. Fd-based set-len (SYS_TRUNCATE(18)
/// is path-based). The fd must be writable (EINVAL otherwise, per Linux).
pub unsafe fn sys_fs_ftruncate(fd: u64, new_len: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();
    let desc = match fd_table.get(fd as usize) {
        Some(d) => *d,
        None => return EBADF,
    };
    if desc.revoked {
        return EBADF;
    }
    if desc.kind != FdKind::Regular {
        return EINVAL;
    }
    let status = fd_table.status_flags(fd as usize).unwrap_or(desc.flags);
    if status & O_WRITE == 0 {
        return EINVAL;
    }
    let ts = fs_now_ns();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (m, dev) = match g.mount_dev_mut(desc.mount_id) {
        Some(t) => t,
        None => return EBADF,
    };
    match m.fs.truncate(dev, desc.path_str(), new_len, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

/// SYS_RMDIR: `path_ptr,path_len -> 0 | -errno`. Directories only; a non-directory
/// target → `ENOTDIR` and a non-empty one → `ENOTEMPTY` (the backend reports the
/// latter). `SYS_UNLINK(16)` stays files-only, so `remove_dir`/`remove_file` map
/// cleanly. Type check + remove share one `STORAGE_LOCK` section (no TOCTOU).
pub unsafe fn sys_fs_rmdir(path_ptr: u64, path_len: u64) -> u64 {
    let path = match resolve_user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let ts = fs_now_ns();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(&path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.stat(dev, rel) {
        Ok(st) if st.mode & mode::S_IFMT == mode::S_IFDIR => {},
        Ok(_) => return ENOTDIR,
        Err(e) => return vfs_err_to_errno(e),
    }
    match m.fs.unlink(dev, rel, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}
