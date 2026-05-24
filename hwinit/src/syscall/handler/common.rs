
const ENOSYS: u64 = u64::MAX - 37;
const EINVAL: u64 = u64::MAX;
const EPERM: u64 = u64::MAX - 1;
const ENOENT: u64 = u64::MAX - 2;
const ESRCH: u64 = u64::MAX - 3;
const EIO: u64 = u64::MAX - 5;
const EBADF: u64 = u64::MAX - 9;
const ENOMEM: u64 = u64::MAX - 12;
const EFAULT: u64 = u64::MAX - 14;
const ENOTDIR: u64 = u64::MAX - 20;
const EPIPE: u64 = u64::MAX - 32;
const EBUSY: u64 = u64::MAX - 16;

/// Canonical lower-half limit (AMD64 Vol 2 §5.3).
const USER_ADDR_LIMIT: u64 = 0x0000_8000_0000_0000;

/// True iff `[ptr, ptr+len)` lies entirely in the lower-half canonical region.
#[inline]
fn validate_user_buf(ptr: u64, len: u64) -> bool {
    if ptr == 0 || len == 0 {
        return false;
    }
    match ptr.checked_add(len) {
        Some(e) => e <= USER_ADDR_LIMIT,
        None => false,
    }
}

unsafe fn user_path(ptr: u64, len: u64) -> Option<&'static str> {
    if ptr == 0 || len == 0 || len > 255 {
        return None;
    }
    if !validate_user_buf(ptr, len) {
        return None;
    }
    let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
    core::str::from_utf8(bytes).ok()
}

fn helix_err_to_errno(_e: morpheus_helix::error::HelixError) -> u64 {
    use morpheus_helix::error::HelixError::*;
    match _e {
        NotFound => ENOENT,
        AlreadyExists => u64::MAX - 17, // EEXIST
        InvalidFd => EBADF,
        TooManyOpenFiles => u64::MAX - 24,  // EMFILE
        ReadOnly => u64::MAX - 30,          // EROFS
        IsADirectory => u64::MAX - 21,      // EISDIR
        DirectoryNotEmpty => u64::MAX - 39, // ENOTEMPTY
        NoSpace => u64::MAX - 28,           // ENOSPC
        MountNotFound => ENOENT,
        PermissionDenied => u64::MAX - 13, // EACCES
        InvalidOffset => EINVAL,
        IoReadFailed | IoWriteFailed | IoFlushFailed => EIO,
        _ => EINVAL,
    }
}

// SMP serialization for FsGlobal. Two cores aliasing the static mut corrupts
// the VFS — confirmed empirically.
static VFS_LOCK: crate::sync::RawSpinLock = crate::sync::RawSpinLock::new();

struct VfsGuard {
    fs: &'static mut morpheus_helix::vfs::global::FsGlobal,
}

impl Drop for VfsGuard {
    fn drop(&mut self) {
        VFS_LOCK.unlock();
    }
}

/// Returns None if FS isn't initialized. Lock drops with the guard.
unsafe fn vfs_lock() -> Option<VfsGuard> {
    VFS_LOCK.lock();
    match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => Some(VfsGuard { fs }),
        None => {
            VFS_LOCK.unlock();
            None
        }
    }
}

