//! USB class drivers + glue. Lives under `morpheus_xhci::usb::*`.
//!
//! Phase 3.7 wave C1c folded the HID class drivers, MSI wiring, and runtime
//! polling loop into morpheus-xhci alongside the controller. Wave C3 retired
//! the hwinit transitional shim entirely; consumers (bootloader) now import
//! directly from `morpheus_xhci::usb::*`.
//!
//! - `hid/` — boot-protocol keyboard / mouse report parsers + the sink hooks
//!   the kernel installs at boot.
//! - `msi.rs` — MSI / MSI-X interrupt wiring for the xHCI controller. Reaches
//!   the IDT + LAPIC + PCI MSI capability through `morpheus_kernel::hal()`.
//! - `runtime.rs` — keeps the controller alive past Phase 9 and exposes the
//!   non-blocking `poll_keyboard` the bootloader's input loop calls each
//!   iteration.

pub mod hid;
pub mod msi;
pub mod runtime;

// Back-compat re-exports — the shared types/methods live one level up in
// morpheus-xhci itself.
pub use crate::{
    asm, controller, dma, enum_, enumerate, hid_iface, hub, regs, rings, XhciController,
    XhciDevice, XhciError,
};
