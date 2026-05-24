//! Boot-protocol HID mouse. PS/2 and USB are mutually exclusive (see input.rs).

use crate::input::{self, InputEvent};
use crate::usb::controller::{XhciController, XhciError};
use crate::usb::dma;
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

    push_mouse_event(InputEvent::Move(report.x as i16, report.y as i16));

    // Buttons: bit0 L, bit1 R, bit2 M.
    push_mouse_button(0, (report.buttons & 0x01) != 0);
    push_mouse_button(1, (report.buttons & 0x02) != 0);
    push_mouse_button(2, (report.buttons & 0x04) != 0);

    if report.wheel != 0 {
        push_mouse_event(InputEvent::Wheel(report.wheel));
    }

    Ok(())
}

fn push_mouse_event(event: InputEvent) {
    input::push_mouse_event_internal(event);
}

fn push_mouse_button(button: usize, pressed: bool) {
    if button < 3 {
        push_mouse_event(InputEvent::Button(button, pressed));
    }
}

pub fn register_handler() {
    input::register_mouse_handler(usb_mouse_handler);
}

/// Passthrough — events flow through push_mouse_event_internal directly.
fn usb_mouse_handler(dx: i16, dy: i16, buttons: u8) {
    let _ = dx;
    let _ = dy;
    let _ = buttons;
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
