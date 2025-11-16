//! x86_64 architecture-specific boot code
//!
//! GRUB-compatible boot paths:
//! - EFI Handover Protocol (industry standard)
//! - 32-bit protected mode (legacy fallback)

pub mod handoff;
pub mod transitions;

pub use handoff::BootPath;
pub use transitions::drop_to_protected_mode;
