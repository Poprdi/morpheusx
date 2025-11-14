//! Kernel handoff implementations
//! 
//! Two paths:
//! 1. EFI handoff (64-bit, modern kernels)
//! 2. Protected mode handoff (32-bit, universal compatibility)

use core::arch::asm;

/// EFI 64-bit handoff protocol
/// 
/// For kernels that support it (handover_offset != 0).
/// Stays in 64-bit long mode, kernel handles everything.
/// 
/// Entry point: startup_64 + handover_offset
/// RSI: boot_params pointer
/// 
/// Does NOT return.
#[unsafe(naked)]
pub unsafe extern "C" fn efi_handoff_64(entry_point: u64, boot_params: u64) -> ! {
    core::arch::naked_asm!(
        ".att_syntax",
        
        // Clear interrupts
        "cli",
        
        // Clear direction flag (required)
        "cld",
        
        // Zero all registers except RSI (boot_params)
        "xor %rax, %rax",
        "xor %rbx, %rbx",
        "xor %rcx, %rcx",
        "xor %rdx, %rdx",
        // RSI already contains boot_params (second arg)
        "xor %rdi, %rdi",
        "xor %rbp, %rbp",
        "xor %r8, %r8",
        "xor %r9, %r9",
        "xor %r10, %r10",
        "xor %r11, %r11",
        "xor %r12, %r12",
        "xor %r13, %r13",
        "xor %r14, %r14",
        "xor %r15, %r15",
        
        // Jump to kernel (first arg in RDI)
        "jmp *%rdi",
    )
}

/// Protected mode 32-bit handoff
/// 
/// Universal compatibility path.
/// Assumes we're already in 32-bit protected mode.
/// 
/// Entry point: code32_start from setup header
/// ESI: boot_params pointer
/// 
/// Does NOT return.
#[unsafe(naked)]
pub unsafe extern "C" fn protected_mode_handoff_32(entry_point: u32, boot_params: u32) -> ! {
    core::arch::naked_asm!(
        ".code32",
        ".intel_syntax noprefix",
        
        // Clear interrupts
        "cli",
        
        // Clear direction flag
        "cld",
        
        // Zero all registers except ESI (boot_params)
        "xor eax, eax",
        "xor ebx, ebx",
        "xor ecx, ecx",
        "xor edx, edx",
        // ESI already contains boot_params (second arg)
        "xor edi, edi",
        "xor ebp, ebp",
        
        // Jump to kernel (first arg in EDI)
        "jmp edi",
        
        ".att_syntax",
        ".code64",
    )
}

/// Boot protocol decision logic
/// 
/// Determines which handoff method to use based on:
/// - Current CPU mode (detected at runtime)
/// - Kernel capabilities (handover_offset)
/// - Firmware type (UEFI vs BIOS)
#[derive(Debug, Copy, Clone)]
pub enum BootPath {
    /// Modern: UEFI 64-bit → EFI handoff → kernel 64-bit entry
    EfiHandoff64 { entry: u64 },
    
    /// Universal: Any mode → 32-bit protected → kernel 32-bit entry  
    ProtectedMode32 { entry: u32 },
}

impl BootPath {
    /// Determine optimal boot path
    pub fn choose(
        handover_offset: Option<u32>,
        startup_64: u64,
        code32_start: u32,
        in_long_mode: bool,
    ) -> Self {
        // Prefer EFI handoff if kernel supports it and we're in long mode
        if in_long_mode {
            if let Some(offset) = handover_offset {
                return BootPath::EfiHandoff64 {
                    entry: startup_64 + offset as u64,
                };
            }
        }
        
        // Fallback: 32-bit protected mode (universal)
        BootPath::ProtectedMode32 { entry: code32_start }
    }
    
    /// Execute the handoff (does not return)
    pub unsafe fn execute(self, boot_params: u64) -> ! {
        match self {
            BootPath::EfiHandoff64 { entry } => {
                efi_handoff_64(entry, boot_params)
            }
            BootPath::ProtectedMode32 { entry } => {
                // Need to drop to 32-bit first if we're in 64-bit
                super::transitions::drop_to_protected_mode(entry, boot_params as u32)
            }
        }
    }
}
