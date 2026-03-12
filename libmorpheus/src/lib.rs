//! libmorpheus — userspace syscall library and SDK.
//!
//! # Layers
//!
//! 1. **Raw**: `raw::syscall0`..`syscall5` — thin `syscall` instruction wrappers.
//! 2. **Bare functions**: `fs::open`, `net::tcp_send`, etc. — return `Result<T, u64>`.
//! 3. **Ergonomic types**: `fs::File`, `net::TcpStream`, `time::Instant`, etc. —
//!    RAII, implement `io::Read`/`io::Write`, use `error::Error`.
//!
//! RAX=nr, RDI..R9=args, RAX=return. errors > 0xFFFF_FFFF_FFFF_FF00.

#![no_std]
#![allow(dead_code)]

extern crate alloc; // buddy.rs registers #[global_allocator] → Vec/Box/String work

pub mod buddy;
pub mod compositor;
pub mod desktop;
pub mod entry;
pub mod env;
pub mod error;
pub mod fs;
pub mod hw;
pub mod io;
pub mod mem;
pub mod net;
pub mod persist;
pub mod process;
pub mod raw;
pub mod sync;
pub mod sys;
pub mod task;
pub mod thread;
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
