//! Persistent Layer Management
//!
//! This module handles bootloader self-persistence and data persistence.
//! 
//! # Architecture
//! 
//! The persistence system is divided into platform-neutral and platform-specific code:
//! 
//! ## Platform-Neutral (core logic):
//! - PE/COFF header parsing (DOS, PE, section tables)
//! - Memory image capture
//! - Storage backends (ESP, TPM, CMOS, HVRAM)
//! 
//! ## Platform-Specific (arch modules):
//! - x86_64: PE+ with IMAGE_REL_BASED_DIR64 relocations
//! - aarch64: PE+ with ARM64-specific considerations
//! - armv7: PE with 32-bit relocations (future)
//! 
//! # Critical Design Decision
//! 
//! This is the FIRST point where platform-specific code diverges.
//! The relocation format is identical (PE base relocation table),
//! but the *semantics* differ:
//! 
//! - x86_64: Simple pointer fixups
//! - ARM64: May involve instruction encoding (ADRP/ADD pairs)
//! - ARM32: Thumb mode considerations
//! 
//! We use trait-based abstraction to keep the core agnostic.

#![no_std]

extern crate alloc;

// Public modules
pub mod pe;        // PE/COFF parsing (platform-neutral)
pub mod capture;   // Memory image extraction
pub mod storage;   // Persistence backends
pub mod feedback;  // Visual feedback and logging

// Platform-specific relocation engines
#[cfg(target_arch = "x86_64")]
pub mod arch {
    pub mod x86_64;
}

#[cfg(target_arch = "aarch64")]
pub mod arch {
    pub mod aarch64;
}

// Future: armv7 support
// #[cfg(target_arch = "arm")]
// pub mod arch {
//     pub mod armv7;
// }
