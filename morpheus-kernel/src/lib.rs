//! Arch-agnostic kernel core. All hardware interaction routes through [`hal()`].

#![no_std]
#![allow(static_mut_refs)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::result_unit_err)]

extern crate alloc;

mod global;
pub mod serial;
pub mod sync;

pub mod input;
pub mod mouse;
pub mod pipe;
pub mod ps2_mouse;
pub mod stdin;
pub mod stdout;

pub mod process;
pub mod schedular;
pub mod shutdown;
pub mod syscall;

pub mod elf;
pub mod init;
pub mod sched_hooks;

pub use global::{hal, install_hal};
pub use init::{build_kernel_hooks, init as late_init, InitParams};
