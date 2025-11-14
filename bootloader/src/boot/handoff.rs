// Kernel boot handoff

use super::{KernelImage, LinuxBootParams};

#[cfg(target_arch = "x86_64")]
use super::arch::x86_64::handoff::BootPath;

pub enum HandoffError {
    ExitBootServicesFailed,
    InvalidKernel,
}

// Jump to kernel entry point
// This function does not return!
pub unsafe fn boot_kernel(
    kernel: &KernelImage,
    boot_params: *mut LinuxBootParams,
    _system_table: *mut (),
    kernel_loaded_addr: *mut u8,
) -> ! {
    // x86_64 architecture: choose optimal boot path
    #[cfg(target_arch = "x86_64")]
    {
        use super::arch::x86_64::transitions::check_efi_handover_support;
        
        // Check if kernel supports EFI handover
        let setup_header = kernel.setup_header_bytes();
        let handover_offset = check_efi_handover_support(setup_header);
        
        // For 32-bit mode, we need kernel loaded at code32_start address
        // For EFI mode, kernel can be anywhere (position independent)
        let code32_start = kernel.code32_start();
        
        // Entry points:
        // - EFI handover: kernel_loaded_addr + handover_offset (position independent)
        // - 32-bit mode: kernel_loaded_addr (bzImage decompressor stub starts at offset 0)
        let startup_64 = kernel_loaded_addr as u64;
        let protected_mode_entry = kernel_loaded_addr as u32;
        
        // We're currently in long mode (UEFI bootloader)
        let in_long_mode = true;
        
        // Choose and execute boot path
        let boot_path = BootPath::choose(
            handover_offset,
            startup_64,
            protected_mode_entry,
            in_long_mode
        );
        
        boot_path.execute(boot_params as u64)
    }
    
    // Other architectures: implement as needed
    #[cfg(not(target_arch = "x86_64"))]
    {
        panic!("Unsupported architecture for kernel boot");
    }
}
