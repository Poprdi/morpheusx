//! Boot-protocol HID mouse. PS/2 and USB are mutually exclusive (see input.rs).

use crate::controller::{XhciController, XhciError};
use crate::dma;
use crate::usb::hid::sink;
use crate::usb::hid::HIDInterface;

/// Boot-protocol 4-byte report (HID 1.11 §B.2).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MouseReport {
    pub buttons: u8,
    pub x: i8,
    pub y: i8,
    pub wheel: i8,
}

pub unsafe fn parse_mouse_report(
    _controller: &mut XhciController,
    _iface: &HIDInterface,
    report: *const MouseReport,
) -> Result<(), XhciError> {
    let report = &*(report as *const MouseReport);

    // Push a single composite event to the kernel sink. Kernel translates
    // it into the InputEvent stream (move / button / wheel) as needed.
    let buttons_bits = report.buttons & 0x07;
    sink::push_mouse(
        report.x as i16,
        report.y as i16,
        buttons_bits,
        report.wheel,
    );

    Ok(())
}

/// No-op after Phase 3.2 — the kernel installs the mouse sink at boot.
/// Kept for ABI compatibility with existing callers.
pub fn register_handler() {
    // No-op: see keyboard.rs::register_handler.
}

/// TODO: real interrupt-in handling. Parses whatever's in OFF_REPORT today.
pub unsafe fn handle_interrupt_transfer(
    controller: &mut XhciController,
    iface: &HIDInterface,
) -> Result<(), XhciError> {
    let report_buf = controller.dma_base + dma::OFF_REPORT as u64;
    let report = report_buf as *const MouseReport;
    parse_mouse_report(controller, iface, report)
}
