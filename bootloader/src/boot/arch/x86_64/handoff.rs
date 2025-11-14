//! Kernel handoff implementations
//! 
//! Two paths:
//! 1. EFI handoff (64-bit, modern kernels)
//! 2. Protected mode handoff (32-bit, universal compatibility)

pub const EFI_HANDOVER_ENTRY_BIAS: u64 = 0x200;

/// EFI 64-bit handoff protocol
/// 
/// For kernels that support it (handover_offset != 0).
/// Stays in 64-bit long mode, kernel handles everything.
/// 
/// Entry point: startup_64 + handover_offset
///
/// Win64 calling convention expected by the EFI stub:
/// RCX = image handle
/// RDX = system table
/// R8  = boot_params pointer
/// 
/// Does NOT return.
#[unsafe(naked)]
pub unsafe extern "win64" fn efi_handoff_64(
    entry_point: u64,
    image_handle: u64,
    system_table: u64,
    boot_params: u64,
) -> ! {
    core::arch::naked_asm!(
        "mov r11, rcx",   // stash entry pointer (arg1)
        "mov rcx, rdx",   // RCX = image handle (arg2)
        "mov rdx, r8",    // RDX = system table (arg3)
        "mov r8, r9",     // R8  = boot params (arg4)

        "cli",
        "cld",

        "xor rax, rax",
        "xor rbx, rbx",
        "xor rbp, rbp",
        "xor r10, r10",
        "xor r12, r12",
        "xor r13, r13",
        "xor r14, r14",
        "xor r15, r15",

        "jmp r11",
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
        protected_mode_entry: u32,
        in_long_mode: bool,
    ) -> Self {
        // Prefer EFI handoff if kernel supports it and we're in long mode
        if in_long_mode {
            if let Some(offset) = handover_offset {
                let entry = startup_64 + offset as u64 + EFI_HANDOVER_ENTRY_BIAS;
                return BootPath::EfiHandoff64 { entry };
            }
        }
        
        // Fallback: 32-bit protected mode (universal)
        BootPath::ProtectedMode32 { entry: protected_mode_entry }
    }
    
    /// Execute the handoff (does not return)
    pub unsafe fn execute(self, boot_params: u64, image_handle: *mut (), system_table: *mut ()) -> ! {
        match self {
            BootPath::EfiHandoff64 { entry } => {
                efi_handoff_64(entry, image_handle as u64, system_table as u64, boot_params)
            }
            BootPath::ProtectedMode32 { entry } => {
                // Need to drop to 32-bit first if we're in 64-bit
                super::transitions::drop_to_protected_mode(entry, boot_params as u32)
            }
        }
    }
}
