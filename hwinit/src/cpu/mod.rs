//! CPU Management
//!
//! Complete CPU state management for bare-metal operation.
//!
//! # Modules
//!
//! - `gdt` - Global Descriptor Table and TSS
//! - `idt` - Interrupt Descriptor Table and exception handlers
//! - `pic` - Programmable Interrupt Controller (8259)
//! - `barriers` - Memory barriers
//! - `cache` - Cache management
//! - `mmio` - Memory-mapped I/O
//! - `pio` - Port I/O
//! - `tsc` - Time Stamp Counter

pub mod barriers;
pub mod cache;
pub mod gdt;
pub mod idt;
pub mod mmio;
pub mod pic;
pub mod pio;
pub mod tsc;
