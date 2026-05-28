//! Boot-protocol HID keyboard. Translates HID usage codes to PS/2 Set 1
//! scancodes and feeds the unified input layer (PS/2 or USB, never both —
//! see input.rs).
//!
//! Wire protocol into the unified queue
//! The HID boot report (8 bytes) has no per-key press/release bit; state
//! changes are inferred by diffing the current report against the previous
//! one. From that diff we synthesize raw PS/2 Set 1 bytes and push them as
//! `InputEvent::Key(byte, true)` — the `pressed` flag is repurposed as
//! "process this byte", not "this is a make code." A break is encoded by
//! `(scancode | 0x80)` in the byte, exactly like real PS/2 hardware would
//! send. Extended keys (arrows, F-keys' nav cluster, right modifiers, GUI,
//! menu) push two events: first `Key(0xE0, true)`, then the make/break byte.
//! The bootloader's `Keyboard::feed_raw` reassembles the sequence.

use crate::controller::{XhciController, XhciError};
use crate::dma;
use crate::usb::hid::sink;
use crate::usb::hid::HIDInterface;
use morpheus_kernel::sync::SpinLock;

/// Boot-protocol 8-byte report (HID 1.11 §B.1).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KeyboardReport {
    pub modifiers: u8,
    pub reserved: u8,
    pub keys: [u8; 6],
}

impl KeyboardReport {
    const fn zero() -> Self {
        Self {
            modifiers: 0,
            reserved: 0,
            keys: [0; 6],
        }
    }
}

/// Last report we successfully parsed. The next report is diffed against
/// this to derive press/release events; without it we'd re-fire press
/// events for every held key on every poll.
static PREV_REPORT: SpinLock<KeyboardReport> = SpinLock::new(KeyboardReport::zero());

const EXT_PREFIX: u8 = 0xE0;
const BREAK_FLAG: u8 = 0x80;

/// HID usage codes for the eight standard modifier bits in report byte 0.
/// Bit 0 = LeftCtrl, bit 1 = LeftShift, ..., bit 7 = RightGUI.
const MODIFIER_USAGE: [u8; 8] = [0xE0, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7];

/// # Safety
/// `report` must point to a readable `KeyboardReport` in the DMA buffer.
pub unsafe fn parse_keyboard_report(
    _controller: &mut XhciController,
    _iface: &HIDInterface,
    report: *const KeyboardReport,
) -> Result<(), XhciError> {
    let cur = *report;
    let mut prev = PREV_REPORT.lock();

    // ---- Modifier byte diff (8 independent bits) ----
    let changed = cur.modifiers ^ prev.modifiers;
    if changed != 0 {
        for bit in 0..8u8 {
            let mask = 1u8 << bit;
            if changed & mask == 0 {
                continue;
            }
            let pressed_now = cur.modifiers & mask != 0;
            if pressed_now {
                push_make(MODIFIER_USAGE[bit as usize]);
            } else {
                push_break(MODIFIER_USAGE[bit as usize]);
            }
        }
    }

    // ---- Key array diff ----
    // Release events: usage codes present in prev but not in cur.
    for &k in prev.keys.iter() {
        if k == 0 {
            continue;
        }
        if !contains(&cur.keys, k) {
            push_break(k);
        }
    }
    // Press events: usage codes present in cur but not in prev.
    for &k in cur.keys.iter() {
        if k == 0 {
            continue;
        }
        if !contains(&prev.keys, k) {
            push_make(k);
        }
    }

    *prev = cur;
    Ok(())
}

#[inline]
fn contains(arr: &[u8; 6], needle: u8) -> bool {
    arr.contains(&needle)
}

#[inline]
fn push_byte(byte: u8) {
    // pressed=true is the "process this byte" flag — the consumer drops
    // events with pressed=false. See module-level comment.
    sink::push_keyboard_byte(byte, true);
}

/// Push the Set 1 make sequence for a HID usage code (1 or 2 bytes).
/// Unmapped codes are silently dropped.
fn push_make(hid_code: u8) {
    let (prefix, base) = translate_hid_to_ps2_set1(hid_code);
    if base == 0 {
        return;
    }
    if let Some(p) = prefix {
        push_byte(p);
    }
    push_byte(base);
}

/// Push the Set 1 break sequence for a HID usage code (1 or 2 bytes).
/// Unmapped codes are silently dropped.
fn push_break(hid_code: u8) {
    let (prefix, base) = translate_hid_to_ps2_set1(hid_code);
    if base == 0 {
        return;
    }
    if let Some(p) = prefix {
        push_byte(p);
    }
    push_byte(base | BREAK_FLAG);
}

/// Returns `(extended_prefix, base_scancode)` for a HID usage code.
/// `extended_prefix == Some(0xE0)` means the caller must prefix the base byte
/// with 0xE0. `base_scancode` is the Set 1 make code; OR 0x80 for the break.
/// `(None, 0)` = unmapped — caller should skip emitting anything.
///
/// USB HID Usage Page 0x07 (keyboard) → IBM PC AT scan code set 1.
/// References: HID Usage Tables 1.5 §10, "Keyboard Scan Codes" (IBM).
fn translate_hid_to_ps2_set1(hid_code: u8) -> (Option<u8>, u8) {
    match hid_code {
        // ---- Letters (a–z, HID 0x04..=0x1D) ----
        0x04 => (None, 0x1E), // a
        0x05 => (None, 0x30), // b
        0x06 => (None, 0x2E), // c
        0x07 => (None, 0x20), // d
        0x08 => (None, 0x12), // e
        0x09 => (None, 0x21), // f
        0x0A => (None, 0x22), // g
        0x0B => (None, 0x23), // h
        0x0C => (None, 0x17), // i
        0x0D => (None, 0x24), // j
        0x0E => (None, 0x25), // k
        0x0F => (None, 0x26), // l
        0x10 => (None, 0x32), // m
        0x11 => (None, 0x31), // n
        0x12 => (None, 0x18), // o
        0x13 => (None, 0x19), // p
        0x14 => (None, 0x10), // q
        0x15 => (None, 0x13), // r
        0x16 => (None, 0x1F), // s
        0x17 => (None, 0x14), // t
        0x18 => (None, 0x16), // u
        0x19 => (None, 0x2F), // v
        0x1A => (None, 0x11), // w
        0x1B => (None, 0x2D), // x
        0x1C => (None, 0x15), // y
        0x1D => (None, 0x2C), // z

        // ---- Top-row digits ----
        0x1E => (None, 0x02), // 1
        0x1F => (None, 0x03), // 2
        0x20 => (None, 0x04), // 3
        0x21 => (None, 0x05), // 4
        0x22 => (None, 0x06), // 5
        0x23 => (None, 0x07), // 6
        0x24 => (None, 0x08), // 7
        0x25 => (None, 0x09), // 8
        0x26 => (None, 0x0A), // 9
        0x27 => (None, 0x0B), // 0

        // ---- Editing / whitespace ----
        0x28 => (None, 0x1C), // Enter
        0x29 => (None, 0x01), // Escape
        0x2A => (None, 0x0E), // Backspace
        0x2B => (None, 0x0F), // Tab
        0x2C => (None, 0x39), // Space

        // ---- Punctuation row ----
        0x2D => (None, 0x0C), // - _
        0x2E => (None, 0x0D), // = +
        0x2F => (None, 0x1A), // [ {
        0x30 => (None, 0x1B), // ] }
        0x31 => (None, 0x2B), // \ |
        // 0x32 — Non-US # ~ (rare); leave unmapped for now.
        0x33 => (None, 0x27), // ; :
        0x34 => (None, 0x28), // ' "
        0x35 => (None, 0x29), // ` ~
        0x36 => (None, 0x33), // , <
        0x37 => (None, 0x34), // . >
        0x38 => (None, 0x35), // / ?
        0x39 => (None, 0x3A), // CapsLock

        // ---- Function keys ----
        0x3A => (None, 0x3B), // F1
        0x3B => (None, 0x3C), // F2
        0x3C => (None, 0x3D), // F3
        0x3D => (None, 0x3E), // F4
        0x3E => (None, 0x3F), // F5
        0x3F => (None, 0x40), // F6
        0x40 => (None, 0x41), // F7
        0x41 => (None, 0x42), // F8
        0x42 => (None, 0x43), // F9
        0x43 => (None, 0x44), // F10
        0x44 => (None, 0x57), // F11
        0x45 => (None, 0x58), // F12

        // ---- Navigation / editing cluster (all 0xE0-prefixed) ----
        0x46 => (Some(EXT_PREFIX), 0x37), // PrintScreen (simplified; real PS/2 is E0 2A E0 37)
        // 0x47 ScrollLock, 0x48 Pause/Break — both messy; skip.
        0x49 => (Some(EXT_PREFIX), 0x52), // Insert
        0x4A => (Some(EXT_PREFIX), 0x47), // Home
        0x4B => (Some(EXT_PREFIX), 0x49), // PageUp
        0x4C => (Some(EXT_PREFIX), 0x53), // Delete
        0x4D => (Some(EXT_PREFIX), 0x4F), // End
        0x4E => (Some(EXT_PREFIX), 0x51), // PageDown
        0x4F => (Some(EXT_PREFIX), 0x4D), // RightArrow
        0x50 => (Some(EXT_PREFIX), 0x4B), // LeftArrow
        0x51 => (Some(EXT_PREFIX), 0x50), // DownArrow
        0x52 => (Some(EXT_PREFIX), 0x48), // UpArrow

        // ---- Keypad ----
        0x53 => (None, 0x45),             // NumLock
        0x54 => (Some(EXT_PREFIX), 0x35), // Keypad /
        0x55 => (None, 0x37),             // Keypad *
        0x56 => (None, 0x4A),             // Keypad -
        0x57 => (None, 0x4E),             // Keypad +
        0x58 => (Some(EXT_PREFIX), 0x1C), // Keypad Enter
        0x59 => (None, 0x4F),             // Keypad 1 (End)
        0x5A => (None, 0x50),             // Keypad 2 (Down)
        0x5B => (None, 0x51),             // Keypad 3 (PgDn)
        0x5C => (None, 0x4B),             // Keypad 4 (Left)
        0x5D => (None, 0x4C),             // Keypad 5
        0x5E => (None, 0x4D),             // Keypad 6 (Right)
        0x5F => (None, 0x47),             // Keypad 7 (Home)
        0x60 => (None, 0x48),             // Keypad 8 (Up)
        0x61 => (None, 0x49),             // Keypad 9 (PgUp)
        0x62 => (None, 0x52),             // Keypad 0 (Insert)
        0x63 => (None, 0x53),             // Keypad . (Delete)

        // ---- Application / context menu ----
        0x65 => (Some(EXT_PREFIX), 0x5D), // App / Menu

        // ---- Standalone modifier usages (0xE0..=0xE7).
        // Most keyboards report modifiers via byte 0 of the report, but
        // some quirky firmware also drops them into the keys array; handle
        // both paths via the same translator so encoding stays consistent.
        0xE0 => (None, 0x1D),             // LeftCtrl
        0xE1 => (None, 0x2A),             // LeftShift
        0xE2 => (None, 0x38),             // LeftAlt
        0xE3 => (Some(EXT_PREFIX), 0x5B), // LeftGUI (Win)
        0xE4 => (Some(EXT_PREFIX), 0x1D), // RightCtrl
        0xE5 => (None, 0x36),             // RightShift
        0xE6 => (Some(EXT_PREFIX), 0x38), // RightAlt
        0xE7 => (Some(EXT_PREFIX), 0x5C), // RightGUI

        _ => (None, 0),
    }
}

/// Previously called `input::register_keyboard_handler`. After Phase 3.2 the
/// kernel installs its own keyboard sink via `sink::set_keyboard_sink` at
/// boot; HID parsers push through that sink. This function is now a no-op
/// kept for ABI compatibility with the (few) callers that still invoke it.
pub fn register_handler() {
    // No-op: kernel-side `input::register_keyboard_handler` is wired by the
    // kernel during HAL bring-up. HID parser is sink-driven.
}

/// TODO: real interrupt-in handling. Currently parses whatever's already in OFF_REPORT.
///
/// # Safety
/// `controller` must have valid DMA mappings and the caller must hold exclusive
/// access; the report region must contain a valid `KeyboardReport`.
pub unsafe fn handle_interrupt_transfer(
    controller: &mut XhciController,
    iface: &HIDInterface,
) -> Result<(), XhciError> {
    let report_buf = controller.dma_base + dma::OFF_REPORT as u64;
    let report = report_buf as *const KeyboardReport;
    parse_keyboard_report(controller, iface, report)
}
