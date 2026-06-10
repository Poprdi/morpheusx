//! libmorpheus — userspace syscall library.
//!
//! Layers: `raw` syscall wrappers, bare `Result<T, u64>` helpers, RAII types.
//! ABI: RAX=nr, RDI..R9=args, RAX=return; errors are values > `0xFFFF_FFFF_FFFF_FF00`.

#![no_std]
#![allow(dead_code)]

extern crate alloc; // buddy.rs registers the global allocator

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
    is_error, EACCES, EAGAIN, EBADF, EBUSY, ECHILD, EEXIST, EFAULT, EINVAL, EIO, EISDIR, EMFILE,
    ENODEV, ENOENT, ENOMEM, ENOSPC, ENOSYS, ENOTDIR, ENOTEMPTY, EPERM, EPIPE, EROFS, ESRCH,
};
