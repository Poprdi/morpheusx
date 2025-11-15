//! x86_64 architecture-specific boot code
//! 
//! GRUB-compatible boot paths:
//! - EFI Handover Protocol (industry standard)
//! - 32-bit protected mode (legacy fallback)

pub mod transitions;
pub mod handoff;

pub use transitions::drop_to_protected_mode;
pub use handoff::BootPath;
