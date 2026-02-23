//! libmorpheus — Userspace library for MorpheusX native binaries.
//!
//! Apps link against this crate to access kernel services via syscalls.
//! All functions use the `syscall` instruction with the MorpheusX ABI:
//!   RAX=number, RDI=a1, RSI=a2, RDX=a3, R10=a4, R8=a5, R9=a6
//!   Return: RAX (negative = error)
//!
//! # Quick Start
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! use libmorpheus::entry;
//!
//! entry!(main);
//!
//! fn main() -> i32 {
//!     libmorpheus::io::println("hello from Ring 3!");
//!     0
//! }
//! ```

#![no_std]
#![allow(dead_code)]

pub mod entry;
pub mod fs;
pub mod io;
pub mod process;
pub mod raw;

/// Error values returned by the kernel (high bits of u64).
pub const ENOSYS: u64 = u64::MAX - 37;
pub const EINVAL: u64 = u64::MAX;
pub const ENOMEM: u64 = u64::MAX - 12;
pub const ENOENT: u64 = u64::MAX - 2;

/// Check if a syscall return value is an error.
#[inline]
pub fn is_error(ret: u64) -> bool {
    ret > 0xFFFF_FFFF_FFFF_FF00
}
