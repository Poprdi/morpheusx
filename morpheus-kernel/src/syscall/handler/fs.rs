// VFS syscalls: open/close/seek/stat/readdir/mkdir/unlink/rename/truncate/sync/
// snapshot/versions, plus volumes/mounts/mount/umount. All route through
// `crate::storage` (spec §5/§7): validate → STORAGE_LOCK → resolve → split-borrow
// device → match-dispatch the mount's backend → VfsError→errno.

use super::common::*;
use crate::hal;
use crate::schedular::SCHEDULER;
use crate::storage::{self, vfs_err_to_errno};
use morpheus_foundation::errno::EXDEV;
use morpheus_foundation::flags::open_flags::{O_APPEND, O_PIPE_READ, O_PIPE_WRITE, O_WRITE};
use morpheus_foundation::storage::{MNT_RDONLY, MNT_STAGED};
use morpheus_foundation::syscall_abi::{SEEK_CUR, SEEK_END, SEEK_SET};

pub unsafe fn sys_fs_open(path_ptr: u64, path_len: u64, flags: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let flags = flags as u32;
    let ts = hal().timer().read_tsc();
    let fd_table = SCHEDULER.current_fd_table_mut();

    // Allocate the fd slot first so a full table fails before touching the FS.
    let fd = match fd_table.alloc() {
        Some(fd) => fd,
        None => return EMFILE,
    };

    let guard = storage::lock();
    let g = &mut *guard.g;
    let (mount_id, m, dev, rel) = match g.resolve_mut(path) {
        Some(t) => t,
        None => return ENOENT,
    };

    let opened = match m.fs.open(dev, rel, flags, ts) {
        Ok(o) => o,
        Err(e) => return vfs_err_to_errno(e),
    };

    let mut state = crate::storage::fs_api::FdState::empty();
    state.mount_id = mount_id;
    state.flags = flags;
    // Store the mount-relative path: every later fd op re-derives by path and the
    // backend's namespace is mount-rooted.
    let pb = rel.as_bytes();
    let n = pb.len().min(state.path.len());
    state.path[..n].copy_from_slice(&pb[..n]);
    state.path_len = n as u16;
    state.cookie = opened.cookie;
    // O_APPEND positions at EOF; resolved lazily on first write via stat would
    // need the engine, so seed from the backend stat here.
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

    // Pipes carry no mount; their teardown stays with the pipe layer.
    if desc.flags & (O_PIPE_READ | O_PIPE_WRITE) != 0 {
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
    let desc = match fd_table.get(fd as usize) {
        Some(d) => *d,
        None => return EBADF,
    };
    if desc.revoked {
        return EBADF;
    }

    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize);
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (m, dev) = match g.mount_dev_mut(desc.mount_id) {
        Some(t) => t,
        None => return EBADF,
    };
    match m.fs.read(dev, &desc, buf) {
        Ok(n) => {
            if let Some(d) = fd_table.get_mut(fd as usize) {
                d.offset = d.offset.saturating_add(n as u64);
            }
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
    if desc.flags & O_WRITE == 0 {
        return EBADF;
    }

    let buf = core::slice::from_raw_parts(buf_ptr as *const u8, len as usize);
    let ts = hal().timer().read_tsc();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (m, dev) = match g.mount_dev_mut(desc.mount_id) {
        Some(t) => t,
        None => return EBADF,
    };
    match m.fs.write(dev, &mut desc, buf, ts) {
        Ok(n) => {
            // The backend advanced `desc.offset`; persist it back to the table.
            if let Some(d) = fd_table.get_mut(fd as usize) {
                d.offset = desc.offset;
            }
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

    // SEEK_END needs the current file size from the backend.
    let base = match whence {
        SEEK_SET => 0i64,
        SEEK_CUR => desc.offset as i64,
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
    match fd_table.get_mut(fd as usize) {
        Some(d) => {
            d.offset = new_off as u64;
            new_off as u64
        },
        None => EBADF,
    }
}

pub unsafe fn sys_fs_stat(path_ptr: u64, path_len: u64, stat_buf: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.stat(dev, rel) {
        Ok(stat) => {
            if stat_buf != 0 {
                let size = core::mem::size_of::<morpheus_foundation::types::FileStat>() as u64;
                if !validate_user_buf(stat_buf, size) {
                    return EFAULT;
                }
                *(stat_buf as *mut morpheus_foundation::types::FileStat) = stat;
            }
            0
        },
        Err(e) => vfs_err_to_errno(e),
    }
}

pub unsafe fn sys_fs_readdir(path_ptr: u64, path_len: u64, buf_ptr: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.readdir(dev, rel) {
        Ok(entries) => {
            let count = entries.len();
            if buf_ptr != 0 && count > 0 {
                let entry_size =
                    core::mem::size_of::<morpheus_foundation::types::DirEntry>() as u64;
                let total = (count as u64).saturating_mul(entry_size);
                if !validate_user_buf(buf_ptr, total) {
                    return EFAULT;
                }
                let dst = buf_ptr as *mut morpheus_foundation::types::DirEntry;
                for (i, entry) in entries.iter().enumerate() {
                    *dst.add(i) = *entry;
                }
            }
            count as u64
        },
        Err(e) => vfs_err_to_errno(e),
    }
}

pub unsafe fn sys_fs_mkdir(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let ts = hal().timer().read_tsc();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.mkdir(dev, rel, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

pub unsafe fn sys_fs_unlink(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let ts = hal().timer().read_tsc();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.unlink(dev, rel, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

pub unsafe fn sys_fs_rename(old_ptr: u64, old_len: u64, new_ptr: u64, new_len: u64) -> u64 {
    let old = match user_path(old_ptr, old_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let new = match user_path(new_ptr, new_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let ts = hal().timer().read_tsc();
    let guard = storage::lock();
    let g = &mut *guard.g;

    // rename across mounts is EXDEV, never an implicit copy+delete (spec §4).
    let src_mount = match g.mounts.resolve(old) {
        Some(id) => id,
        None => return ENOENT,
    };
    let dst_mount = match g.mounts.resolve(new) {
        Some(id) => id,
        None => return ENOENT,
    };
    if src_mount != dst_mount {
        return EXDEV;
    }
    // Strip the shared mount prefix from both paths before the backend sees them.
    let mp_len = match g.mounts.get(src_mount) {
        Some(m) => m.mount_point_len as usize,
        None => return ENOENT,
    };
    let rel_old = storage::mount_relative(old, mp_len);
    let rel_new = storage::mount_relative(new, mp_len);
    let (m, dev) = match g.mount_dev_mut(src_mount) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.rename(dev, rel_old, rel_new, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

/// Resize `path` to `new_size`: shrink truncates, grow zero-extends. A backend
/// that rejects an oversized grow surfaces its own error.
pub unsafe fn sys_fs_truncate(path_ptr: u64, path_len: u64, new_size: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let ts = hal().timer().read_tsc();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.truncate(dev, rel, new_size, ts) {
        Ok(()) => 0,
        Err(e) => vfs_err_to_errno(e),
    }
}

/// Sync every mounted backend against its device.
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

/// Records a named snapshot marker on the mount holding `name`'s path-root and
/// returns the backend handle (Helix LSN). An empty name is an anonymous
/// checkpoint of the root mount.
pub unsafe fn sys_fs_snapshot(name_ptr: u64, name_len: u64) -> u64 {
    let name = if name_ptr == 0 || name_len == 0 {
        ""
    } else {
        match user_path(name_ptr, name_len) {
            Some(p) => p,
            None => return EINVAL,
        }
    };
    // Snapshot targets the root mount (the snapshot name is a marker, not a path).
    let ts = hal().timer().read_tsc();
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

/// Fills `buf` with up to `max` `FileVersion` records for `path` (oldest first)
/// and returns the number written. A null/zero buffer with `max == 0` returns
/// the available version count without writing (probe convention).
pub unsafe fn sys_fs_versions(path_ptr: u64, path_len: u64, buf_ptr: u64, max: u64) -> u64 {
    use morpheus_foundation::types::FileVersion;

    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(path) {
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

/// `volumes(buf_ptr, max) -> count` (SYS_VOLUMES). Probe-then-fill: `max == 0`
/// returns the count without writing; otherwise fills `min(count, max)`
/// `VolumeInfo` records. Mirrors the `SYS_VERSIONS` convention.
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
        };
    }
    count as u64
}

/// `mounts(buf_ptr, max) -> count` (SYS_MOUNTS). Same probe-then-fill convention
/// as `sys_volumes`.
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
            _pad: [0u8; 6],
        };
    }
    count as u64
}

/// `mount(source_volume_id, mp_ptr, mp_len, fs_type, flags, aux)` (SYS_MOUNT,
/// spec §5). `source_volume_id == VOLUME_NONE` mounts a fresh RAM (tmpfs);
/// `MNT_STAGED` copies a real volume into RAM. Returns `mount_id` or errno.
/// `aux` = required size when source is VOLUME_NONE, else optional stage cap.
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

    // Userland-driven mounts are charged against the calling pid's RAM budget and
    // never run privileged (spec §6).
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

/// `umount(mp_ptr, mp_len, flags)` (SYS_UMOUNT, spec §5). Exact mountpoint only;
/// `MNT_FORCE` revokes open fds. Returns 0 or errno.
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
