//! Shared helpers + errno constants for syscall handlers. The filesystem path
//! now routes through `crate::storage` (spec §7); handlers acquire the subsystem
//! via `storage::lock()` directly.

// Canonical errno values live in morpheus-foundation — single source of truth.
pub(crate) use morpheus_foundation::errno::{
    EAGAIN, EBADF, EBUSY, EFAULT, EINVAL, EIO, EMFILE, ENODEV, ENOENT, ENOMEM, ENOSYS, ENOTDIR,
    EPERM, EPIPE, ESRCH,
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
