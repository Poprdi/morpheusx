//! Kernel handoff implementations
//! 
//! Based on GRUB bootloader (the industry standard):
//! EFI Handover Protocol - used by all modern Linux kernels
//! 
//! Entry: kernel_addr + handover_offset + 512 (x86_64)
//! Calling convention: handover_func(image_handle, system_table, boot_params)

/// x86_64 offset added to handover_offset
pub const EFI_HANDOVER_OFFSET_X64: u64 = 512;

/// EFI Handover Protocol entry
/// 
/// Based on GRUB's implementation. This is THE standard way to boot Linux kernels.
/// 
/// Entry point: kernel_addr + handover_offset + 512 (for x86_64)
/// 
/// Calling convention (from GRUB):
/// typedef void (*handover_func)(void *image_handle, 
///                               grub_efi_system_table_t *systab, 
///                               void *boot_params);
/// 
/// Does NOT return.
#[unsafe(naked)]
pub unsafe extern "C" fn efi_handover_boot(
    entry_point: u64,      // RDI - handover entry address
    image_handle: u64,     // RSI - EFI image handle
    system_table: u64,     // RDX - EFI system table
    boot_params: u64,      // RCX - Linux boot_params
) -> ! {
    core::arch::naked_asm!(
        // GRUB-style EFI handover protocol
        // We receive: RDI=entry, RSI=handle, RDX=systab, RCX=params
        // Handover expects: RCX=handle, RDX=systab, R8=params (Win64 ABI!)
        
        "mov r11, rdi",      // Save entry point
        "mov rcx, rsi",      // RCX = image_handle (arg1 Win64)
        "mov rdx, rdx",      // RDX = system_table (arg2 Win64) - already correct
        "mov r8, rcx",       // R8 = boot_params (arg3 Win64) - from our RCX
        
        "cli",
        "cld",
        
        // Jump to handover entry point
        "jmp r11",
    )
}

/// Boot protocol decision logic
/// 
/// Based on GRUB bootloader approach:
/// Use EFI handover protocol when handover_offset != 0
/// Otherwise fall back to 32-bit protected mode
#[derive(Debug, Copy, Clone)]
pub enum BootPath {
    /// EFI Handover Protocol (GRUB-style, industry standard)
    /// Entry: kernel_addr + handover_offset + 512
    EfiHandover64 { entry: u64 },
    
    /// Legacy: 32-bit protected mode fallback
    ProtectedMode32 { entry: u32 },
}

impl BootPath {
    /// Determine boot path (GRUB-compatible)
    /// 
    /// GRUB uses handover_offset when available, regardless of kernel version.
    /// This works for kernels 3.x through current 6.x.
    pub fn choose(
        handover_offset: Option<u32>,
        startup_64: u64,
        protected_mode_entry: u32,
        in_long_mode: bool,
    ) -> Self {
        if in_long_mode {
            // Use EFI handover protocol if kernel supports it
            if let Some(offset) = handover_offset {
                if offset != 0 {
                    // GRUB formula: kernel_addr + handover_offset + 512 (x86_64)
                    let entry = startup_64 + offset as u64 + EFI_HANDOVER_OFFSET_X64;
                    return BootPath::EfiHandover64 { entry };
                }
            }
        }
        
        // Fallback: 32-bit protected mode
        BootPath::ProtectedMode32 { entry: protected_mode_entry }
    }
    
    /// Execute the handoff (does not return)
    pub unsafe fn execute(self, boot_params: u64, image_handle: *mut (), system_table: *mut ()) -> ! {
        match self {
            BootPath::EfiHandover64 { entry } => {
                // GRUB-style EFI handover: pass image_handle, system_table, boot_params
                efi_handover_boot(
                    entry,
                    image_handle as u64,
                    system_table as u64,
                    boot_params,
                )
            }
            BootPath::ProtectedMode32 { entry } => {
                // Drop to 32-bit protected mode
                super::transitions::drop_to_protected_mode(entry, boot_params as u32)
            }
        }
    }
}
