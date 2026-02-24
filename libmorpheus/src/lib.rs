//! libmorpheus — userspace syscall library.
//! RAX=nr, RDI..R9=args, RAX=return. errors > 0xFFFF_FFFF_FFFF_FF00.

#![no_std]
#![allow(dead_code)]

extern crate alloc; // buddy.rs registers #[global_allocator] → Vec/Box/String work

pub mod buddy;
pub mod entry;
pub mod fs;
pub mod hw;
pub mod io;
pub mod mem;
pub mod net;
pub mod persist;
pub mod process;
pub mod raw;
pub mod sys;
pub mod time;

/// Error codes from kernel. high bits = bad news.
pub const ENOSYS: u64 = u64::MAX - 37;
pub const EINVAL: u64 = u64::MAX;
pub const ENOMEM: u64 = u64::MAX - 12;
pub const ENOENT: u64 = u64::MAX - 2;
pub const EBADF: u64 = u64::MAX - 9;
pub const EPIPE: u64 = u64::MAX - 32;
pub const EFAULT: u64 = u64::MAX - 14;
pub const ESRCH: u64 = u64::MAX - 3;
pub const EIO: u64 = u64::MAX - 5;

/// true if the kernel is telling you to go away.
#[inline]
pub fn is_error(ret: u64) -> bool {
    ret > 0xFFFF_FFFF_FFFF_FF00
}
