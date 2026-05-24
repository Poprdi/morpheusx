//! USB subsystem — xHCI host controller + HID input enumeration.
//!
//! Used exclusively during Phase 9 of platform init (before the scheduler
//! is live) to discover and bind USB keyboard/mouse devices.

pub mod asm;
pub mod bot;
pub mod control;
pub mod controller;
pub mod dma;
pub mod enumerate;
pub mod hid;
pub mod regs;
pub mod rings;

// `enum` is a Rust keyword; the file is named `enum.rs` but we import it
// under an alias so the rest of the crate can refer to it as `crate::usb::enum_`.
#[path = "enum.rs"]
pub mod enum_;
