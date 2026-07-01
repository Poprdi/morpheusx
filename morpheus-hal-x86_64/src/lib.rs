//! x86_64 HAL impl. Kernel never imports this crate; it gets `&'static dyn Hal`
//! from the bootloader.

#![no_std]

pub mod asm;
pub mod cpu;
pub mod dma;
pub mod heap;
pub mod intr;
pub mod io;
pub mod memory;
pub mod paging;
pub mod pci;
pub mod platform;
pub mod rtc;
pub mod serial;
pub mod sync;

mod hal_impl;
pub use hal_impl::HalImpl;
