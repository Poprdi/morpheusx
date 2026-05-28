//! HID report parsers (keyboard -> PS/2 scancode, mouse delta). Class-request
//! methods and `HIDInterface`/`USB_*` constants live in `crate::hid_iface`,
//! re-exported below for `usb::hid::*` callers.

pub mod keyboard;
pub mod mouse;
pub mod sink;

pub use crate::hid_iface::{
    HIDInterface, USB_CLASS_HID, USB_PROTOCOL_KEYBOARD, USB_PROTOCOL_MOUSE, USB_SUBCLASS_BOOT,
};
