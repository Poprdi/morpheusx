//! PS/2 mouse driver — STUB.
//!
//! The bootloader event loop (`bootloader/src/tui/desktop.rs`) owns PS/2
//! mouse polling via `asm_ps2_poll_any()` and the `Mouse` decoder in
//! `bootloader/src/tui/mouse.rs`.  This module previously contained a
//! duplicate driver that polled port 0x60 directly from the scheduler
//! tick, causing conflicts:
//!   - Competing reads on port 0x60 could steal bytes from the other driver
//!   - Config byte writes here disabled keyboard IRQs (`& !0x01`)
//!   - Double Y-axis flip (both drivers negated dy independently)
//!
//! Both `init()` and `poll()` are now no-ops.  The bootloader driver is
//! the single source of truth for all PS/2 I/O.

/// No-op — mouse init is done by `bootloader/src/tui/mouse.rs`.
pub fn init() {}

/// No-op — mouse polling is done by the bootloader event loop.
pub fn poll() {}
