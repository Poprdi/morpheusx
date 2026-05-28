//! USB class drivers + glue: HID report parsers, MSI wiring, runtime polling.

pub mod hid;
pub mod msi;
pub mod runtime;

// Re-exports of shared types living one level up in the crate root.
pub use crate::{
    asm, controller, dma, enum_, enumerate, hid_iface, hub, regs, rings, XhciController,
    XhciDevice, XhciError,
};
