//! Shared helpers and errno constants for syscall handlers (storage: spec §7).

use alloc::string::String;
use alloc::vec::Vec;

pub(crate) use morpheus_foundation::errno::{
    EAGAIN, EBADF, EBUSY, EEXIST, EFAULT, EINVAL, EIO, EISDIR, EMFILE, ENODEV, ENOENT, ENOMEM,
    ENOSYS, ENOTDIR, EPERM, EPIPE, ESRCH,
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

/// Canonicalize an already-absolute path; `..` at root is clamped, never escapes `/`.
pub(crate) fn normalize_abs(input: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in input.split('/') {
        match seg {
            "" | "." => {},
            ".." => {
                parts.pop();
            },
            s => parts.push(s),
        }
    }
    let mut out = String::with_capacity(input.len() + 1);
    out.push('/');
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(p);
    }
    out
}

/// Resolve a user path to a canonical absolute path: relative paths join the
/// caller's cwd (not the root mount). `None` on a bad user pointer.
pub(crate) unsafe fn resolve_user_path(ptr: u64, len: u64) -> Option<String> {
    let p = user_path(ptr, len)?;
    if p.starts_with('/') {
        return Some(normalize_abs(p));
    }
    let cwd = crate::schedular::SCHEDULER.current_process_mut().cwd_str();
    let mut joined = String::from(cwd);
    if !joined.ends_with('/') {
        joined.push('/');
    }
    joined.push_str(p);
    Some(normalize_abs(&joined))
}

/// FS timestamp source (mtime/atime, log records). Monotonic ns-since-boot, not
/// wall-clock: CLOCK_REALTIME epoch base awaits Domain F, and `FileStat` documents
/// these fields as monotonic until then.
#[inline]
pub(crate) fn fs_now_ns() -> u64 {
    crate::global::hal().timer().now_ns()
}

/// Fill the std-required metadata the backends leave at zero: the `{version,
/// struct_size}` self-describing head and a sane `nlink` (POSIX link count is
/// never 0 for a live object). `uid`/`gid` stay 0 (single-user root) by design.
pub(crate) fn fill_stat_metadata(stat: &mut morpheus_foundation::types::FileStat) {
    use morpheus_foundation::flags::mode;
    stat.version = 1;
    stat.struct_size = core::mem::size_of::<morpheus_foundation::types::FileStat>() as u16;
    if stat.nlink == 0 {
        stat.nlink = if stat.mode & mode::S_IFMT == mode::S_IFDIR {
            2
        } else {
            1
        };
    }
}
