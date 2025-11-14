//! Kernel handoff implementations
//! 
//! Three paths:
//! 1. EFI Stub (64-bit, modern kernels 6.1+) - Direct startup_64
//! 2. EFI Handover (64-bit, kernels 3.x-6.0) - startup_64 + handover_offset + 0x200
//! 3. Protected mode (32-bit, legacy kernels) - code32_start

pub const EFI_HANDOVER_ENTRY_BIAS: u64 = 0x200;

/// EFI 64-bit stub entry
/// 
/// For modern Linux kernels, jump to startup_64 + 0x200 with:
/// - RSI = pointer to boot_params
/// - All other registers zeroed
/// 
/// This is the System V AMD64 ABI, NOT Win64!
/// 
/// Does NOT return.
#[unsafe(naked)]
pub unsafe extern "C" fn efi_stub_64(
    entry_point: u64,      // RDI
    boot_params: u64,      // RSI  
) -> ! {
    core::arch::naked_asm!(
        // Parameters (System V AMD64 ABI):
        // RDI = entry_point
        // RSI = boot_params (already in the right register!)
        
        "mov rax, rdi",    // Save entry point to RAX
        "xor rdi, rdi",    // Zero RDI as per Linux spec
        // RSI already has boot_params
        
        "cli",
        "cld",
        
        // Zero all other registers as per Linux kernel requirements
        "xor rbx, rbx",
        "xor rcx, rcx",
        "xor rdx, rdx",
        "xor rbp, rbp",
        "xor r8, r8",
        "xor r9, r9",
        "xor r10, r10",
        "xor r11, r11",
        "xor r12, r12",
        "xor r13, r13",
        "xor r14, r14",
        "xor r15, r15",
        
        "jmp rax",         // Jump to kernel entry point
    )
}

/// Boot protocol decision logic
/// 
/// Determines which handoff method to use based on:
/// - Kernel version/capabilities (handover_offset, xloadflags)
/// - Current CPU mode (64-bit long mode vs 32-bit)
#[derive(Debug, Copy, Clone)]
pub enum BootPath {
    /// Modern (6.1+): UEFI 64-bit → Direct EFI stub → kernel startup_64
    EfiStub64 { entry: u64 },
    
    /// Legacy EFI (3.x-6.0): UEFI 64-bit → EFI handover → kernel 64-bit entry
    EfiHandover64 { entry: u64 },
    
    /// Legacy: Any mode → 32-bit protected → kernel code32_start
    ProtectedMode32 { entry: u32 },
}

impl BootPath {
    /// Determine optimal boot path
    /// 
    /// Based on Limine bootloader's proven approach:
    /// - 64-bit kernels with XLF_KERNEL_64 | XLF_CAN_BE_LOADED_ABOVE_4G: Use startup_64 + 0x200
    /// - Legacy kernels: Fall back to 32-bit protected mode with code32_start
    pub fn choose(
        xloadflags: u16,
        startup_64: u64,
        protected_mode_entry: u32,
        in_long_mode: bool,
    ) -> Self {
        if in_long_mode {
            // Check for 64-bit kernel support (XLF_KERNEL_64 | XLF_CAN_BE_LOADED_ABOVE_4G)
            // This is the modern and correct way, used by Limine and other production bootloaders
            if (xloadflags & 3) == 3 {
                // Jump to startup_64 + 0x200 (EFI handoff entry point)
                // This works for ALL kernels - both modern (6.x) and older (3.x-5.x)
                return BootPath::EfiStub64 { entry: startup_64 + EFI_HANDOVER_ENTRY_BIAS };
            }
        }
        
        // Fallback: 32-bit protected mode (legacy kernels)
        BootPath::ProtectedMode32 { entry: protected_mode_entry }
    }
    
    /// Execute the handoff (does not return)
    pub unsafe fn execute(self, boot_params: u64, image_handle: *mut (), system_table: *mut ()) -> ! {
        match self {
            BootPath::EfiStub64 { entry } | BootPath::EfiHandover64 { entry } => {
                // Modern Linux EFI stub expects System V AMD64 ABI:
                // RDI = entry_point, RSI = boot_params
                efi_stub_64(entry, boot_params)
            }
            BootPath::ProtectedMode32 { entry } => {
                // Need to drop to 32-bit first if we're in 64-bit
                super::transitions::drop_to_protected_mode(entry, boot_params as u32)
            }
        }
    }
}
