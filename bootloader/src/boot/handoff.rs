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
) -> ! {
    // x86_64 architecture: choose optimal boot path
    #[cfg(target_arch = "x86_64")]
    {
        use super::arch::x86_64::transitions::check_efi_handover_support;
        
        // Check if kernel supports EFI handover
        let setup_header = kernel.setup_header_bytes();
        let handover_offset = check_efi_handover_support(setup_header);
        
        // Determine entry points
        let startup_64 = kernel.kernel_base() as u64;
        let code32_start = kernel.code32_start();
        
        // We're currently in long mode (UEFI bootloader)
        let in_long_mode = true;
        
        // Choose and execute boot path
        let boot_path = BootPath::choose(
            handover_offset,
            startup_64,
            code32_start,
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
