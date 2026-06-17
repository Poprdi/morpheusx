//! Canonical syscall error codes — the single source of truth for both the
//! kernel handlers and userland.
//!
//! Convention: a syscall returns its result in the low range, or an error as
//! `u64::MAX - k` (so errors occupy the very top of the u64 range). `is_error`
//! defines the boundary. These MUST match on both sides of the seam, so they
//! live here and nowhere else — kernel `common.rs` and `libmorpheus` re-export
//! these rather than re-declaring them.

pub const EINVAL: u64 = u64::MAX; // -1: invalid argument
pub const EPERM: u64 = u64::MAX - 1; // operation not permitted
pub const ENOENT: u64 = u64::MAX - 2; // no such file/entry
pub const ESRCH: u64 = u64::MAX - 3; // no such process
pub const EIO: u64 = u64::MAX - 5; // I/O error
pub const EBADF: u64 = u64::MAX - 9; // bad file descriptor
pub const ECHILD: u64 = u64::MAX - 10; // no child processes (wait)
pub const EAGAIN: u64 = u64::MAX - 11; // try again / would block
pub const ENOMEM: u64 = u64::MAX - 12; // out of memory
pub const EACCES: u64 = u64::MAX - 13; // permission denied
pub const EFAULT: u64 = u64::MAX - 14; // bad address
pub const EBUSY: u64 = u64::MAX - 16; // resource busy
pub const EEXIST: u64 = u64::MAX - 17; // already exists
pub const EXDEV: u64 = u64::MAX - 18; // cross-device link (rename across mounts)
pub const ENODEV: u64 = u64::MAX - 19; // no such device
pub const ENOTDIR: u64 = u64::MAX - 20; // not a directory
pub const EISDIR: u64 = u64::MAX - 21; // is a directory
pub const EMFILE: u64 = u64::MAX - 24; // too many open files
pub const ENOSPC: u64 = u64::MAX - 28; // no space left
pub const EROFS: u64 = u64::MAX - 30; // read-only filesystem
pub const EPIPE: u64 = u64::MAX - 32; // broken pipe
pub const ENOSYS: u64 = u64::MAX - 37; // not implemented
pub const ENOTEMPTY: u64 = u64::MAX - 39; // directory not empty

/// True iff `ret` encodes an error (top of the u64 range). Matches the kernel's
/// `u64::MAX - k` convention; the gap below leaves room for real large returns.
#[inline]
pub const fn is_error(ret: u64) -> bool {
    ret > 0xFFFF_FFFF_FFFF_FF00
}
