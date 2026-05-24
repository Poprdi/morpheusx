//! Boot-protocol HID keyboard. Translates HID usage codes to PS/2 scancodes
//! and feeds the unified input layer (PS/2 or USB, never both — see input.rs).

use crate::input::{self, InputEvent};
use crate::usb::controller::{XhciController, XhciError};
use crate::usb::dma;
use crate::usb::hid::HIDInterface;

/// Boot-protocol 8-byte report (HID 1.11 §B.1).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KeyboardReport {
    pub modifiers: u8,
    pub reserved: u8,
    pub keys: [u8; 6],
}

pub unsafe fn parse_keyboard_report(
    _controller: &mut XhciController,
    _iface: &HIDInterface,
    report: *const KeyboardReport,
) -> Result<(), XhciError> {
    let report = &*(report as *const KeyboardReport);

    // Modifier byte: LCtrl/LShift/LAlt/LWin/RCtrl/RShift/RAlt/RWin (bits 0..=7).
    let modifiers = report.modifiers;
    push_key(InputEvent::Key(0x1D, (modifiers & 0x01) != 0));
    push_key(InputEvent::Key(0x2A, (modifiers & 0x02) != 0));
    push_key(InputEvent::Key(0x38, (modifiers & 0x04) != 0));
    push_key(InputEvent::Key(0x5B, (modifiers & 0x08) != 0));
    push_key(InputEvent::Key(0x1D, (modifiers & 0x10) != 0));
    push_key(InputEvent::Key(0x36, (modifiers & 0x20) != 0));
    push_key(InputEvent::Key(0x38, (modifiers & 0x40) != 0));
    push_key(InputEvent::Key(0x5C, (modifiers & 0x80) != 0));

    for key in report.keys.iter() {
        if *key != 0 {
            let scan_code = translate_hid_to_ps2(*key);
            if scan_code != 0 {
                push_key(InputEvent::Key(scan_code, true));
            }
        }
    }

    Ok(())
}

fn push_key(event: InputEvent) {
    input::push_keyboard_event_internal(event);
}

/// Unifies scancode space across PS/2 and USB. HID usage tables §10.
fn translate_hid_to_ps2(hid_code: u8) -> u8 {
    match hid_code {
        // Letters
        0x04 => 0x1C, // a/A
        0x05 => 0x32, // b/B
        0x06 => 0x21, // c/C
        0x07 => 0x23, // d/D
        0x08 => 0x24, // e/E
        0x09 => 0x2B, // f/F
        0x0A => 0x34, // g/G
        0x0B => 0x33, // h/H
        0x0C => 0x43, // i/I
        0x0D => 0x3B, // j/J
        0x0E => 0x42, // k/K
        0x0F => 0x4B, // l/L
        0x10 => 0x3A, // m/M
        0x11 => 0x31, // n/N
        0x12 => 0x44, // o/O
        0x13 => 0x4D, // p/P
        0x14 => 0x15, // q/Q
        0x15 => 0x2D, // r/R
        0x16 => 0x1B, // s/S
        0x17 => 0x2C, // t/T
        0x18 => 0x3C, // u/U
        0x19 => 0x2A, // v/V
        0x1A => 0x1D, // w/W
        0x1B => 0x22, // x/X
        0x1C => 0x35, // y/Y
        0x1D => 0x1A, // z/Z
        // Numbers
        0x1E => 0x02, // 1/!
        0x1F => 0x03, // 2/@
        0x20 => 0x04, // 3/#
        0x21 => 0x05, // 4/$
        0x22 => 0x06, // 5/%
        0x23 => 0x07, // 6/^
        0x24 => 0x08, // 7/&
        0x25 => 0x09, // 8/*
        0x26 => 0x0A, // 9/(
        0x27 => 0x0B, // 0/)
        // Special keys
        0x28 => 0x1C, // Enter
        0x29 => 0x01, // Escape
        0x2A => 0x0E, // Backspace
        0x2B => 0x0F, // Tab
        0x2C => 0x39, // Space
        0x2D => 0x0C, // -/_
        0x2E => 0x0D, // =/+
        0x2F => 0x1A, // [/{
        0x30 => 0x1B, // ]/}
        0x31 => 0x2B, // \/|
        0x32 => 0x29, // `/~
        0x33 => 0x36, // ;/:
        0x34 => 0x37, // '/"
        0x35 => 0x4E, // ,/< (or -/_ for keypad)
        0x36 => 0x6E, // ./> (or + for keypad)
        0x37 => 0x0D, // //? (or * for keypad)
        // Function keys
        0x3A => 0x3B, // F1
        0x3B => 0x3C, // F2
        0x3C => 0x3D, // F3
        0x3D => 0x3E, // F4
        0x3E => 0x3F, // F5
        0x3F => 0x40, // F6
        0x40 => 0x41, // F7
        0x41 => 0x42, // F8
        0x42 => 0x43, // F9
        0x43 => 0x44, // F10
        0x44 => 0x57, // F11
        0x45 => 0x58, // F12
        // Navigation
        0x49 => 0x52, // Insert
        0x4A => 0x47, // Home
        0x4B => 0x49, // Page Up
        0x4C => 0x53, // Delete (Note: 0x4C is actually '5' on keypad)
        0x4E => 0x4A, // Page Down
        0x4F => 0x4D, // Right Arrow
        0x50 => 0x4B, // Left Arrow
        0x51 => 0x50, // Down Arrow
        0x52 => 0x48, // Up Arrow
        // Keypad
        0x53 => 0x37, // Num Lock
        0x54 => 0x4E, // Keypad /
        0x55 => 0x4C, // Keypad *
        0x56 => 0x4A, // Keypad -
        0x57 => 0x4E, // Keypad +
        0x58 => 0x5A, // Keypad Enter
        0x59 => 0x4B, // Keypad 1 (End)
        0x5A => 0x4C, // Keypad 2 (Down)
        0x5B => 0x4D, // Keypad 3 (Page Down)
        0x5C => 0x47, // Keypad 4 (Home)
        0x5D => 0x4C, // Keypad 5
        0x5E => 0x4F, // Keypad 6 (Right)
        0x5F => 0x47, // Keypad 7 (Home)
        0x60 => 0x48, // Keypad 8 (Up)
        0x61 => 0x49, // Keypad 9 (Page Up)
        0x62 => 0x35, // Keypad 0 (Insert)
        0x63 => 0x53, // Keypad . (Delete)
        _ => 0x00,    // Unknown
    }
}

pub fn register_handler() {
    input::register_keyboard_handler(usb_keyboard_handler);
}

/// Passthrough — events flow through push_keyboard_event_internal directly.
fn usb_keyboard_handler(scan_code: u8, pressed: bool) {
    let _ = scan_code;
    let _ = pressed;
}

/// TODO: real interrupt-in handling. Currently parses whatever's already in OFF_REPORT.
pub unsafe fn handle_interrupt_transfer(
    controller: &mut XhciController,
    iface: &HIDInterface,
) -> Result<(), XhciError> {
    let report_buf = controller.dma_base + dma::OFF_REPORT as u64;
    let report = report_buf as *const KeyboardReport;
    parse_keyboard_report(controller, iface, report)
}
