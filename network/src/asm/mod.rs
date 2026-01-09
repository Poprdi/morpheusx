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

// TODO: Conditional compilation based on target and features

pub mod core;
pub mod pci;
pub mod drivers;
