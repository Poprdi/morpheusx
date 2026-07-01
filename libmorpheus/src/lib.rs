//! libmorpheus — userspace syscall library.
//!
//! Layers: `raw` syscall wrappers, bare `Result<T, u64>` helpers, RAII types.
//! ABI: RAX=nr, RDI..R9=args, RAX=return; errors are values > `0xFFFF_FFFF_FFFF_FF00`.

#![no_std]
#![allow(dead_code)]

extern crate alloc; // buddy.rs registers the global allocator

pub mod abi;
pub mod buddy;
pub mod compositor;
pub mod desktop;
pub mod entry;
pub mod env;
pub mod error;
pub mod fs;
pub mod hw;
pub mod io;
pub mod log;
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

// Kernel error codes + `is_error` are canonical in morpheus-foundation — single
// source of truth across the syscall seam. Re-exported to keep `libmorpheus::EINVAL`
// etc. paths stable.
pub use morpheus_foundation::errno::{
    errno_value, is_error, EACCES, EADDRINUSE, EADDRNOTAVAIL, EAGAIN, EBADF, EBUSY, ECHILD,
    ECONNABORTED, ECONNREFUSED, ECONNRESET, EEXIST, EFAULT, EHOSTUNREACH, EINPROGRESS, EINTR,
    EINVAL, EIO, EISDIR, EMFILE, ENETUNREACH, ENODEV, ENOENT, ENOMEM, ENOSPC, ENOSYS, ENOTCONN,
    ENOTDIR, ENOTEMPTY, ENOTSOCK, EPERM, EPIPE, EROFS, ESRCH, ETIMEDOUT, EWOULDBLOCK,
};
