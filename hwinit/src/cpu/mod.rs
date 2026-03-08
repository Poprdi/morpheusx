//! CPU Management
//!
//! Complete CPU state management for bare-metal operation.
//!
//! # Modules
//!
//! - `gdt` - Global Descriptor Table and TSS
//! - `idt` - Interrupt Descriptor Table and exception handlers
//! - `pic` - Programmable Interrupt Controller (8259)
//! - `apic` - Local APIC driver (timer, IPI, EOI)
//! - `per_cpu` - Per-CPU data via GS-base (SMP)
//! - `ap_boot` - Application Processor startup (INIT/SIPI)
//! - `acpi` - ACPI MADT parser for SMP topology
//! - `barriers` - Memory barriers
//! - `cache` - Cache management
//! - `mmio` - Memory-mapped I/O
//! - `pio` - Port I/O
//! - `tsc` - Time Stamp Counter

pub mod acpi;
pub mod ap_boot;
pub mod apic;
pub mod barriers;
pub mod cache;
pub mod gdt;
pub mod idt;
pub mod mmio;
pub mod per_cpu;
pub mod pic;
pub mod pio;
pub mod sse;
pub mod tsc;
