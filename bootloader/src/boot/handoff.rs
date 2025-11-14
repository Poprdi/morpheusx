// Kernel boot handoff

use super::{KernelImage, LinuxBootParams};

#[cfg(target_arch = "x86_64")]
use super::arch::x86_64::handoff::{efi_handoff_64, EFI_HANDOVER_ENTRY_BIAS};

pub enum HandoffError {
    ExitBootServicesFailed,
    InvalidKernel,
}

// Jump to kernel entry point
// This function does not return!
pub unsafe fn boot_kernel(
    kernel: &KernelImage,
    boot_params: *mut LinuxBootParams,
    system_table: *mut (),
    image_handle: *mut (),
    kernel_loaded_addr: *mut u8,
) -> ! {
    // x86_64 architecture: choose optimal boot path
    #[cfg(target_arch = "x86_64")]
    {
        // Check if kernel supports EFI handoff
        if kernel.supports_efi_handover_64() {
            // Calculate entry point directly without enum
            let handover_offset = kernel.handover_offset() as u64;
            let startup_64 = kernel_loaded_addr as u64;
            let entry = startup_64 + handover_offset + EFI_HANDOVER_ENTRY_BIAS;
            
            // Call efi_handoff_64 directly
            efi_handoff_64(entry, image_handle as u64, system_table as u64, boot_params as u64)
        } else {
            // Fallback: 32-bit protected mode
            let protected_mode_entry = kernel_loaded_addr as u32;
            super::arch::x86_64::transitions::drop_to_protected_mode(protected_mode_entry, boot_params as u32)
        }
    }
    
    // Other architectures: implement as needed
    #[cfg(not(target_arch = "x86_64"))]
    {
        panic!("Unsupported architecture for kernel boot");
    }
}
