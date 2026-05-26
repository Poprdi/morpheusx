//! Arch-agnostic FFI-stable types and ABI constants shared across crates.

#![no_std]

extern crate alloc;

pub mod error;
pub mod syscall_abi;
pub mod types;

/// Static array bound for per-CPU bookkeeping; HAL `Smp::max_cpus()` clamps to this.
pub const MAX_CPUS: u32 = 64;

/// 4 KiB physical page size. Fixed across targeted archs.
pub const PAGE_SIZE: u64 = 4096;
