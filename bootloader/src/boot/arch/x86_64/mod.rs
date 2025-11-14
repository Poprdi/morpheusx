//! x86_64 architecture-specific boot code
//! 
//! Handles multiple boot paths:
//! - UEFI 64-bit → Linux kernel (EFI stub or EFI handover)
//! - BIOS 32-bit → Linux kernel (protected mode handoff)
//! - Future: 16-bit real mode → Linux kernel (full legacy path)

pub mod transitions;
pub mod handoff;

pub use transitions::drop_to_protected_mode;
pub use handoff::efi_stub_64;
