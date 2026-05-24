//! Boot-protocol HID mouse. PS/2 and USB are mutually exclusive (see input.rs).

use crate::input::{self, InputEvent};
use crate::usb::controller::{XhciController, XhciError};
use crate::usb::dma;
use crate::usb::hid::HIDInterface;

/// Mouse report structure (standard 4-byte boot report)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MouseReport {
    pub buttons: u8,
    pub x: i8,
    pub y: i8,
    pub wheel: i8,
}

/// Parse USB HID mouse report and push events to the unified input layer.
pub unsafe fn parse_mouse_report(
    _controller: &mut XhciController,
    _iface: &HIDInterface,
    report: *const MouseReport,
) -> Result<(), XhciError> {
    let report = &*(report as *const MouseReport);

    // Process mouse movement
    push_mouse_event(InputEvent::Move(report.x as i16, report.y as i16));

    // Process button events
    // Button 0 = Left
    push_mouse_button(0, (report.buttons & 0x01) != 0);
    // Button 1 = Right
    push_mouse_button(1, (report.buttons & 0x02) != 0);
    // Button 2 = Middle
    push_mouse_button(2, (report.buttons & 0x04) != 0);

    // Process wheel movement
    if report.wheel != 0 {
        push_mouse_event(InputEvent::Wheel(report.wheel));
    }

    Ok(())
}

/// Push a mouse event to the unified input layer.
fn push_mouse_event(event: InputEvent) {
    input::push_mouse_event_internal(event);
}

/// Push a mouse button event.
fn push_mouse_button(button: usize, pressed: bool) {
    if button < 3 {
        push_mouse_event(InputEvent::Button(button, pressed));
    }
}

/// Register this mouse driver with the unified input layer.
pub fn register_handler() {
    // This function is called during enumeration to register
    // the USB mouse handler as a valid input source
    input::register_mouse_handler(usb_mouse_handler);
}

/// USB mouse event handler for the unified input system.
/// Called when USB mouse data is received.
fn usb_mouse_handler(dx: i16, dy: i16, buttons: u8) {
    // The unified input system already handles this
    // This is a passthrough for legacy compatibility
    let _ = dx;
    let _ = dy;
    let _ = buttons;
}

/// Handle mouse input event from USB HID interrupt endpoint.
/// Reads the mouse report and parses it into input events.
pub unsafe fn handle_interrupt_transfer(
    controller: &mut XhciController,
    iface: &HIDInterface,
) -> Result<(), XhciError> {
    let report_buf = controller.dma_base + dma::OFF_REPORT as u64;

    // Submit interrupt IN transfer to read mouse report
    // This is a blocking read for simplicity during boot enumeration

    // TODO: Implement proper interrupt transfer handling

    // Simulate processing a report buffer
    let report = report_buf as *const MouseReport;
    parse_mouse_report(controller, iface, report)
}
