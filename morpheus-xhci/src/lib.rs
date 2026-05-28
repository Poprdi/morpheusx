//! Shared xHCI host controller, consumed by HID input and the USB-MSD storage
//! stack. usb/msi.rs reaches back to the HAL via `morpheus_kernel::hal()`.

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

// `enum` is a Rust keyword; file is enum.rs, aliased to `enum_`.
#[path = "enum.rs"]
pub mod enum_;

pub use controller::{XhciController, XhciError};
pub use enum_::{ep0_max_packet, pack_setup};
pub use enumerate::{enumerate_and_bind_inputs, InputEnumerationResult, UsbInputDevice};
pub use hid_iface::{
    HIDInterface, USB_CLASS_HID, USB_PROTOCOL_KEYBOARD, USB_PROTOCOL_MOUSE, USB_SUBCLASS_BOOT,
};

/// Handle to an enumerated USB device shared by class consumers (HID, BOT).
#[derive(Debug, Clone, Copy)]
pub struct XhciDevice {
    pub slot_id: u8,
    pub dci_in: u8,
    pub dci_out: u8,
    pub mps: u16,
    pub speed: u8,
}
