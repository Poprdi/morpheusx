//! Bare x86_64 asm primitives: PIO, MMIO, barriers, cache, TSC, PAUSE.
//!
//! Leaf crate; zero deps. Thin Rust wrappers over `asm/cpu/*.s`. Non-x86_64
//! stubs let host `cargo check` succeed without nasm.

#![no_std]

pub mod barriers;
pub mod cache;
pub mod delay;
pub mod mmio;
pub mod pio;
pub mod tsc;
