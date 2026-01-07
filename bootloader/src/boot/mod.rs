// Boot module - handles kernel loading and boot handoff

pub mod boot_params;
pub mod efi_stub;
pub mod handoff;
pub mod iso_boot;
pub mod kernel_loader;
pub mod loader;
pub mod memory;

// Architecture-specific boot code
#[cfg(target_arch = "x86_64")]
pub mod arch;

pub use boot_params::LinuxBootParams;
pub use handoff::boot_kernel;
pub use iso_boot::{boot_from_iso, default_cmdline_for_iso, IsoBootError};
pub use kernel_loader::KernelImage;
pub use loader::boot_linux_kernel;
pub use memory::{
    allocate_boot_params, allocate_cmdline, allocate_kernel_memory, load_kernel_image,
};
