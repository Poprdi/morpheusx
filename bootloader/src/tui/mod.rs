//! PS/2 input drivers used by the boot input-forwarding loop.
//!
//! The post-refactor bootloader no longer has a kernel-mode TUI. These
//! modules are pure driver helpers invoked by `boot::stage_e2_*`.

pub mod input;
pub mod mouse;
