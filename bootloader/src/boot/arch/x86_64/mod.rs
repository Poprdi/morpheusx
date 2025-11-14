//! x86_64 architecture-specific boot code
//! 
//! Handles multiple boot paths:
//! - UEFI 64-bit → Linux kernel (EFI handoff or mode switch)
//! - BIOS 32-bit → Linux kernel (direct handoff)
//! - Future: 16-bit real mode → Linux kernel (full legacy path)

pub mod transitions;
pub mod handoff;

pub use transitions::{drop_to_protected_mode, setup_32bit_gdt};
pub use handoff::{efi_handoff_64, protected_mode_handoff_32};
