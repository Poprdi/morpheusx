// VFS syscalls: open/close/seek/stat/readdir/mkdir/unlink/rename/truncate/sync/snapshot/versions.

use super::common::*;
use crate::hal;
use crate::pipe;
use crate::schedular::SCHEDULER;
use morpheus_helix::types::open_flags::{O_PIPE_READ, O_PIPE_WRITE};

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
    let ts = hal().timer().read_tsc();

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

pub unsafe fn sys_fs_close(fd: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();
    if let Ok(desc) = fd_table.get(fd as usize) {
        let pipe_idx = desc.mount_idx;
        if desc.flags & O_PIPE_READ != 0 {
            pipe::pipe_close_reader(pipe_idx);
        }
        if desc.flags & O_PIPE_WRITE != 0 {
            pipe::pipe_close_writer(pipe_idx);
        }
    }
    match morpheus_helix::vfs::vfs_close(fd_table, fd as usize) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

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
        },
        Err(e) => helix_err_to_errno(e),
    }
}

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
        },
        Err(e) => helix_err_to_errno(e),
    }
}

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
    let ts = hal().timer().read_tsc();
    match morpheus_helix::vfs::vfs_mkdir(&mut fs.mount_table, path, ts) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

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
    let ts = hal().timer().read_tsc();
    match morpheus_helix::vfs::vfs_unlink(&mut fs.mount_table, path, ts) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
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
    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    let ts = hal().timer().read_tsc();
    match morpheus_helix::vfs::vfs_rename(&mut fs.mount_table, old, new, ts) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// Hack: opens with O_TRUNC and closes — always truncates to 0.
/// `new_size` ignored until HelixFS exposes truncate(n).
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
    let ts = hal().timer().read_tsc();

    let flags: u32 = 0x02 | 0x04 | 0x10; // O_WRITE | O_CREATE | O_TRUNC

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

    let _ = new_size; // TODO: extend with zeros once VFS supports truncate(n)
    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
    0
}

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

/// Returns TSC as the snapshot ID. Real log-structured snapshots TBD.
pub unsafe fn sys_fs_snapshot(name_ptr: u64, name_len: u64) -> u64 {
    let _name = user_path(name_ptr, name_len);

    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;

    if let Err(e) = morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table) {
        return helix_err_to_errno(e);
    }
    hal().timer().read_tsc()
}

/// TODO: wire through once VFS exposes vfs_versions().
/// `FileVersion` = { lsn: u64, size: u64, op: u32, _pad: u32 }.
pub unsafe fn sys_fs_versions(path_ptr: u64, path_len: u64, buf_ptr: u64, max: u64) -> u64 {
    let _path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    if buf_ptr == 0 || max == 0 {
        return 0;
    }

    0
}
