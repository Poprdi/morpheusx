//! ASM bindings module - Thin wrappers over standalone assembly functions.
//!
//! This module provides type-safe Rust bindings to the ASM layer.
//! All hardware access goes through these bindings.
//!
//! # Module Organization
//! - `core/` - Core primitives (TSC, barriers, MMIO, PIO, cache)
//! - `pci/` - PCI configuration space access
//! - `drivers/` - Driver-specific ASM bindings
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2, ARCHITECTURE_V3.md

pub mod core;
pub mod pci;
pub mod drivers;

// Re-exports for convenience
#[cfg(target_arch = "x86_64")]
pub use self::core::tsc::read_tsc;
#[cfg(target_arch = "x86_64")]
pub use self::core::barriers::{sfence, lfence, mfence};
#[cfg(target_arch = "x86_64")]
pub use self::core::mmio::{read32 as mmio_read32, write32 as mmio_write32};
#[cfg(target_arch = "x86_64")]
pub use self::core::pio::{inb, outb, inw, outw, inl, outl};
