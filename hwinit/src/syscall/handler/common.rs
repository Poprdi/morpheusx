
// HelixFS Syscall Implementations

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

// USER-POINTER VALIDATION

/// Maximum canonical user virtual address (lower-half).
const USER_ADDR_LIMIT: u64 = 0x0000_8000_0000_0000;

/// Validate a user pointer + length.
///
/// Returns `true` if the range `[ptr .. ptr+len)` is entirely in the
/// user-accessible lower half of the canonical address space and does
/// not wrap around.  Returns `false` (and the syscall should return
/// `-EFAULT`) otherwise.
#[inline]
fn validate_user_buf(ptr: u64, len: u64) -> bool {
    if ptr == 0 || len == 0 {
        return false;
    }
    let end = ptr.checked_add(len);
    match end {
        Some(e) => e <= USER_ADDR_LIMIT,
        None => false, // overflow
    }
}

/// Extract a path `&str` from a user pointer+length, with validation.
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

// smp: serializes all filesystem operations. without this, two cores get aliased
// &mut FsGlobal from the static mut and the whole vfs corrupts itself. ask me how i know.
static VFS_LOCK: crate::sync::RawSpinLock = crate::sync::RawSpinLock::new();

// raii guard — unlocks VFS_LOCK on drop so early returns don't leak the lock.
struct VfsGuard {
    fs: &'static mut morpheus_helix::vfs::global::FsGlobal,
}

impl Drop for VfsGuard {
    fn drop(&mut self) {
        VFS_LOCK.unlock();
    }
}

/// Acquire exclusive vfs access. returns None if fs not initialized.
/// lock auto-releases when the guard drops — early returns are safe.
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

