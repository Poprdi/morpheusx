/// `SYS_OPEN(path_ptr, path_len, flags) → fd`
pub unsafe fn sys_fs_open(path_ptr: u64, path_len: u64, flags: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        flags as u32,
        ts,
    ) {
        Ok(fd) => fd as u64,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_CLOSE(fd) → 0`
pub unsafe fn sys_fs_close(fd: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();
    // Pipe-aware close: decrement refcounts on pipe ends.
    if let Ok(desc) = fd_table.get(fd as usize) {
        let pipe_idx = desc.mount_idx;
        if desc.flags & O_PIPE_READ != 0 {
            crate::pipe::pipe_close_reader(pipe_idx);
        }
        if desc.flags & O_PIPE_WRITE != 0 {
            crate::pipe::pipe_close_writer(pipe_idx);
        }
    }
    match morpheus_helix::vfs::vfs_close(fd_table, fd as usize) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_SEEK(fd, offset, whence) → new_offset`
pub unsafe fn sys_fs_seek(fd: u64, offset: u64, whence: u64) -> u64 {
    let _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &*_vfs_guard.fs;
    let fd_table = SCHEDULER.current_fd_table_mut();
    match morpheus_helix::vfs::vfs_seek(
        &fs.mount_table,
        fd_table,
        fd as usize,
        offset as i64,
        whence,
    ) {
        Ok(pos) => pos,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_STAT(path_ptr, path_len, stat_buf_ptr) → 0`
pub unsafe fn sys_fs_stat(path_ptr: u64, path_len: u64, stat_buf: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &*_vfs_guard.fs;
    match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
        Ok(stat) => {
            if stat_buf != 0 {
                if !validate_user_buf(
                    stat_buf,
                    core::mem::size_of::<morpheus_helix::types::FileStat>() as u64,
                ) {
                    return EFAULT;
                }
                let dst = stat_buf as *mut morpheus_helix::types::FileStat;
                *dst = stat;
            }
            0
        }
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_READDIR(path_ptr, path_len, buf_ptr) → count`
pub unsafe fn sys_fs_readdir(path_ptr: u64, path_len: u64, buf_ptr: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &*_vfs_guard.fs;
    match morpheus_helix::vfs::vfs_readdir(&fs.mount_table, path) {
        Ok(entries) => {
            let count = entries.len();
            if buf_ptr != 0 && count > 0 {
                let entry_size = core::mem::size_of::<morpheus_helix::types::DirEntry>() as u64;
                let total_size = (count as u64).saturating_mul(entry_size);
                if !validate_user_buf(buf_ptr, total_size) {
                    return EFAULT;
                }
                let dst = buf_ptr as *mut morpheus_helix::types::DirEntry;
                for (i, entry) in entries.iter().enumerate() {
                    *dst.add(i) = *entry;
                }
            }
            count as u64
        }
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_MKDIR(path_ptr, path_len) → 0`
pub unsafe fn sys_fs_mkdir(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    let ts = crate::cpu::tsc::read_tsc();
    match morpheus_helix::vfs::vfs_mkdir(&mut fs.mount_table, path, ts) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_UNLINK(path_ptr, path_len) → 0`
pub unsafe fn sys_fs_unlink(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    let ts = crate::cpu::tsc::read_tsc();
    match morpheus_helix::vfs::vfs_unlink(&mut fs.mount_table, path, ts) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_RENAME(old_ptr, old_len, new_ptr, new_len) → 0`
pub unsafe fn sys_fs_rename(old_ptr: u64, old_len: u64, new_ptr: u64, new_len: u64) -> u64 {
    let old = match user_path(old_ptr, old_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let new = match user_path(new_ptr, new_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    let ts = crate::cpu::tsc::read_tsc();
    match morpheus_helix::vfs::vfs_rename(&mut fs.mount_table, old, new, ts) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_TRUNCATE(path_ptr, path_len, new_size) → 0`
///
/// Truncate the file at `path` to `new_size` bytes.
/// Currently implemented as: open with O_WRITE|O_TRUNC, close.
/// This effectively truncates to 0, then writes nothing — the file becomes empty.
/// A proper VFS truncate(new_size) would need HelixFS support.
pub unsafe fn sys_fs_truncate(path_ptr: u64, path_len: u64, new_size: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    // O_WRITE | O_CREATE | O_TRUNC
    let flags: u32 = 0x02 | 0x04 | 0x10;
    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        flags,
        ts,
    ) {
        Ok(fd) => fd,
        Err(e) => return helix_err_to_errno(e),
    };

    // If new_size > 0, we cannot truly extend/truncate to arbitrary size
    // without VFS support — for now, close immediately (file is truncated to 0).
    let _ = new_size; // TODO: write zeros if new_size > 0 once VFS supports it

    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
    0
}

/// `SYS_SYNC() → 0`
pub unsafe fn sys_fs_sync() -> u64 {
    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    match morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_SNAPSHOT(name_ptr, name_len) → snapshot_id`
///
/// Create a filesystem checkpoint. Currently implemented as a full VFS sync
/// with the TSC value returned as the checkpoint identifier.
/// Future: integrate with HelixFS log-structured snapshots.
pub unsafe fn sys_fs_snapshot(name_ptr: u64, name_len: u64) -> u64 {
    // Validate name (optional, for labeling the snapshot).
    let _name = user_path(name_ptr, name_len);

    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;

    // Sync all dirty data to disk.
    if let Err(e) = morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table) {
        return helix_err_to_errno(e);
    }

    // Return TSC as the snapshot ID / checkpoint marker.
    crate::cpu::tsc::read_tsc()
}

/// `SYS_VERSIONS(path_ptr, path_len, buf_ptr, max) → count`
///
/// List version history of a file. Each version entry is a `FileVersion`
/// struct (24 bytes): { lsn: u64, size: u64, op: u32, _pad: u32 }.
///
/// If `buf_ptr` is 0 or `max` is 0, returns the total number of versions.
/// HelixFS supports log-structured versioning via `ops::read::list_versions`.
pub unsafe fn sys_fs_versions(path_ptr: u64, path_len: u64, buf_ptr: u64, max: u64) -> u64 {
    let _path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    // HelixFS VFS layer doesn't expose list_versions() yet.
    // The lower-level helix ops::read::list_versions() exists but requires
    // direct block_io + log access which bypasses the VFS mount table.
    // TODO: wire through VFS once vfs_versions() is implemented.
    if buf_ptr == 0 || max == 0 {
        return 0; // No versions available through VFS yet
    }

    0 // 0 versions written
}
