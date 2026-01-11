//! Kernel handoff implementations
//!
//! Based on GRUB bootloader (the industry standard):
//! EFI Handover Protocol - used by Linux kernels 2.6.30 through 6.0
//!
//! Entry: kernel_addr + handover_offset + 512 (x86_64)
//! Calling convention: handover_func(image_handle, system_table, boot_params)
//!
//! GRUB just casts to function pointer and calls - compiler handles ABI conversion!

/// x86_64 offset added to handover_offset (from Linux kernel docs)
pub const EFI_HANDOVER_OFFSET_X64: u64 = 512;

/// EFI Handover Protocol function signature
///
/// CRITICAL: Uses Microsoft x64 calling convention (Win64 ABI), NOT System V!
/// Parameters: RCX = image_handle, RDX = system_table, R8 = boot_params
///
/// GRUB uses efi_call_3 wrapper to handle calling convention
type HandoverFunc = unsafe extern "efiapi" fn(*mut (), *mut (), *mut ()) -> !;

/// Boot protocol decision logic
///
/// Based on GRUB bootloader approach:
/// - EFI handover protocol when handover_offset != 0
///   → Bootloader does NOT call ExitBootServices (kernel does it)
/// - 32-bit protected mode fallback otherwise
///   → Bootloader MUST call ExitBootServices before jumping
#[derive(Debug, Copy, Clone)]
pub enum BootPath {
    /// EFI Handover Protocol
    /// Entry: kernel_addr + handover_offset + 512
    /// Boot services MUST be active (kernel will exit them)
    EfiHandover64 { entry: u64 },

    /// Legacy: 32-bit protected mode fallback
    /// Boot services MUST be exited before calling
    ProtectedMode32 { entry: u32 },
}

impl BootPath {
    /// Determine boot path to use
    ///
    /// GRUB uses handover_offset when available, regardless of kernel version.
    /// This works for kernels 3.x through 6.0 (handover removed in 6.1).
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
        BootPath::ProtectedMode32 {
            entry: protected_mode_entry,
        }
    }

    /// Execute the handoff (does not return)
    ///
    /// GRUB approach: Cast entry to function pointer and call directly.
    /// Compiler handles System V → Win64 ABI conversion automatically!
    pub unsafe fn execute(
        self,
        boot_params: u64,
        image_handle: *mut (),
        system_table: *mut (),
    ) -> ! {
        match self {
            BootPath::EfiHandover64 { entry } => {
                // GRUB-style: cast to function pointer
                let handover: HandoverFunc = core::mem::transmute(entry);

                // Call with image_handle, system_table, boot_params (GRUB order)
                handover(image_handle, system_table, boot_params as *mut ())
            }
            BootPath::ProtectedMode32 { entry } => {
                // Drop to 32-bit protected mode
                super::transitions::drop_to_protected_mode(entry, boot_params as u32)
            }
        }
    }
}
