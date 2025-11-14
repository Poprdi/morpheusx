// Boot module - handles kernel loading and boot handoff

pub mod kernel_loader;
pub mod boot_params;
pub mod handoff;
pub mod memory;
pub mod loader;

// Architecture-specific boot code
#[cfg(target_arch = "x86_64")]
pub mod arch;

pub use kernel_loader::KernelImage;
pub use boot_params::LinuxBootParams;
pub use handoff::boot_kernel;
pub use memory::{allocate_kernel_memory, allocate_boot_params, allocate_cmdline, load_kernel_image};
pub use loader::boot_linux_kernel;
