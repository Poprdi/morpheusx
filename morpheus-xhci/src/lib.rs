//! Shared xHCI host controller. Consumed by hwinit (HID input) and the storage
//! stack (USB-MSD). Replaces the dual implementation that previously lived in
//! `hwinit/src/usb/` and `network/src/driver/usb_msd/mod.rs`.
//!
//! Phase 2 step 2.1 — extracted from hwinit/usb and network/usb_msd.
//!
//! Class-specific code:
//! - `usb/hid/` — keyboard / mouse report parsers (scancode translation,
//!   mouse delta math). Phase 3.7 wave C1c moved this here from hwinit.
//! - `usb/runtime.rs` — runtime HID polling loop the bootloader's input loop
//!   calls each iteration (Phase 3.7 wave C1c).
//! - `usb/msi.rs` — MSI / MSI-X interrupt wiring for the xHCI controller
//!   (Phase 3.7 wave C1c). Reaches back to the HAL via `morpheus_kernel::hal()`.
//! - `morpheus-block/src/usb_class/` — USB-MSD class wrapper for the runtime
//!   storage driver. Consumes [`XhciController`] directly.
//!
//! See `~/.claude/projects/.../memory/usb_subsystem_overview.md` and
//! `arch_phase2_impl.md` §2.1 for the full architectural intent.

#![no_std]

pub mod asm;
pub mod controller;
pub mod dma;
pub mod enumerate;
pub mod hid_iface;
pub mod hub;
pub mod logger;
pub mod regs;
pub mod rings;
pub mod usb;

// `enum` is a Rust keyword; the file is named `enum.rs` but we import it
// under an alias so the rest of the crate can refer to it as
// `crate::enum_` / `morpheus_xhci::enum_`.
#[path = "enum.rs"]
pub mod enum_;

// Top-level re-exports for the public API surface called out in
// `arch_phase2_impl.md §2.1`.
pub use controller::{XhciController, XhciError};
pub use enum_::{ep0_max_packet, pack_setup};
pub use enumerate::{enumerate_and_bind_inputs, InputEnumerationResult, UsbInputDevice};
pub use hid_iface::{
    HIDInterface, USB_CLASS_HID, USB_PROTOCOL_KEYBOARD, USB_PROTOCOL_MOUSE, USB_SUBCLASS_BOOT,
};

/// Abstract handle to an enumerated USB device. The class-specific consumers
/// (HID, BOT) operate against this rather than re-deriving slot/DCI by hand.
#[derive(Debug, Clone, Copy)]
pub struct XhciDevice {
    pub slot_id: u8,
    pub dci_in: u8,
    pub dci_out: u8,
    pub mps: u16,
    pub speed: u8,
}
