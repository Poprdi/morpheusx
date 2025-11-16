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
    system_table: *mut (),
    image_handle: *mut (),
    kernel_loaded_addr: *mut u8,
) -> ! {
    // x86_64 architecture: GRUB-compatible boot path
    #[cfg(target_arch = "x86_64")]
    {
        // Use handover_offset like GRUB does
        let handover_offset = if kernel.supports_efi_handover_64() {
            Some(kernel.handover_offset())
        } else {
            None
        };

        let startup_64 = kernel_loaded_addr as u64;
        let protected_mode_entry = kernel.code32_start();
        let in_long_mode = true;

        let boot_path = BootPath::choose(
            handover_offset,
            startup_64,
            protected_mode_entry,
            in_long_mode,
        );

        boot_path.execute(boot_params as u64, image_handle, system_table)
    }

    // Other architectures: implement as needed
    #[cfg(not(target_arch = "x86_64"))]
    {
        panic!("Unsupported architecture for kernel boot");
    }
}
