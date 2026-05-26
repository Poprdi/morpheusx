//! HID (Human Interface Device) USB interface implementation
//!
//! HID *report* parsers (keyboard → PS/2 scancode, mouse delta) live here.
//! HID *class-request* methods (`set_hid_idle`, `set_hid_protocol_boot`,
//! `find_hid_interface`, `get_hid_report_descriptor`) and the
//! `HIDInterface` / `USB_*` constants live in `crate::hid_iface` — they're
//! consumed by the shared enumeration walker.
//!
//! This module re-exports them for callers that import via `usb::hid::*`.

pub mod keyboard;
pub mod mouse;
pub mod sink;

pub use crate::hid_iface::{
    HIDInterface, USB_CLASS_HID, USB_PROTOCOL_KEYBOARD, USB_PROTOCOL_MOUSE, USB_SUBCLASS_BOOT,
};
