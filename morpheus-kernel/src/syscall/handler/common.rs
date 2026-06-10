//! Shared helpers + errno constants for syscall handlers.
//!
//! `common.rs` is the SOLE caller of `morpheus_helix::vfs::global::fs_global_mut()`
//! per LD27. All other handler files reach the VFS through `vfs_lock()` or
//! `current_fd_table_and_fs`.

use morpheus_helix::vfs::global::FsGlobal;
use morpheus_helix::vfs::FdTable;

// Canonical errno values live in morpheus-foundation — single source of truth.
pub(crate) use morpheus_foundation::errno::{
    EACCES, EAGAIN, EBADF, EBUSY, EEXIST, EFAULT, EINVAL, EIO, EISDIR, EMFILE, ENODEV, ENOENT,
    ENOMEM, ENOSPC, ENOSYS, ENOTDIR, ENOTEMPTY, EPERM, EPIPE, EROFS, ESRCH,
};

/// Canonical lower-half limit (AMD64 Vol 2 §5.3). Same split applies on
/// ARM HALs; arch-specific deviation belongs in HAL paging helpers, not
/// the syscall boundary.
pub(crate) const USER_ADDR_LIMIT: u64 = 0x0000_8000_0000_0000;

/// True iff `[ptr, ptr+len)` lies entirely in the lower-half canonical region.
#[inline]
pub(crate) fn validate_user_buf(ptr: u64, len: u64) -> bool {
    if ptr == 0 || len == 0 {
        return false;
    }
    match ptr.checked_add(len) {
        Some(e) => e <= USER_ADDR_LIMIT,
        None => false,
    }
}

pub(crate) unsafe fn user_path(ptr: u64, len: u64) -> Option<&'static str> {
    if ptr == 0 || len == 0 || len > 255 {
        return None;
    }
    if !validate_user_buf(ptr, len) {
        return None;
    }
    let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
    core::str::from_utf8(bytes).ok()
}

pub(crate) fn helix_err_to_errno(_e: morpheus_helix::error::HelixError) -> u64 {
    use morpheus_helix::error::HelixError::*;
    match _e {
        NotFound => ENOENT,
        AlreadyExists => EEXIST,
        InvalidFd => EBADF,
        TooManyOpenFiles => EMFILE,
        ReadOnly => EROFS,
        IsADirectory => EISDIR,
        DirectoryNotEmpty => ENOTEMPTY,
        NoSpace => ENOSPC,
        MountNotFound => ENOENT,
        PermissionDenied => EACCES,
        InvalidOffset => EINVAL,
        IoReadFailed | IoWriteFailed | IoFlushFailed => EIO,
        _ => EINVAL,
    }
}

// SMP serialization for FsGlobal. Two cores aliasing the static mut corrupts
// the VFS — confirmed empirically.
pub(crate) static VFS_LOCK: crate::sync::RawSpinLock = crate::sync::RawSpinLock::new();

pub(crate) struct VfsGuard {
    pub fs: &'static mut FsGlobal,
}

impl Drop for VfsGuard {
    fn drop(&mut self) {
        VFS_LOCK.unlock();
    }
}

/// Returns None if FS isn't initialized. Lock drops with the guard.
pub(crate) unsafe fn vfs_lock() -> Option<VfsGuard> {
    VFS_LOCK.lock();
    match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => Some(VfsGuard { fs }),
        None => {
            VFS_LOCK.unlock();
            None
        },
    }
}

/// Unlocked variant of `vfs_lock` paired with the current fd table, for
/// callers already holding `VFS_LOCK`. Returns `None` if FS isn't initialized.
///
/// # Safety
/// Caller must ensure no other core touches the FS or fd table concurrently.
#[allow(dead_code)]
pub(crate) unsafe fn current_fd_table_and_fs(
) -> Option<(&'static mut FdTable, &'static mut FsGlobal)> {
    let fs = morpheus_helix::vfs::global::fs_global_mut()?;
    let fd_table = crate::schedular::SCHEDULER.current_fd_table_mut();
    Some((fd_table, fs))
}
