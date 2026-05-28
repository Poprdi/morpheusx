//! Event-sink callbacks for the HID drivers. The kernel registers
//! keyboard/mouse callbacks at boot; HID parsers invoke them, avoiding a
//! concrete dep on the kernel input module.
//!
//! Callbacks must be re-entrancy safe (driver context, not ISR today, but
//! the boundary is intentionally narrow).

/// Keyboard sink (PS/2 Set 1 scancodes). `pressed` is repurposed as
/// "process this byte" per the keyboard driver convention.
pub type KeyboardSinkFn = fn(byte: u8, pressed: bool);

/// Mouse-event sink: (dx, dy, buttons bitmap, wheel).
pub type MouseSinkFn = fn(dx: i16, dy: i16, buttons: u8, wheel: i8);

static mut KEYBOARD_SINK: Option<KeyboardSinkFn> = None;
static mut MOUSE_SINK: Option<MouseSinkFn> = None;

/// Install the keyboard sink. Single-threaded boot only.
///
/// # Safety
/// Caller guarantees no concurrent HID parsing is running.
pub unsafe fn set_keyboard_sink(f: KeyboardSinkFn) {
    KEYBOARD_SINK = Some(f);
}

/// Install the mouse sink. Single-threaded boot only.
///
/// # Safety
/// Caller guarantees no concurrent HID parsing is running.
pub unsafe fn set_mouse_sink(f: MouseSinkFn) {
    MOUSE_SINK = Some(f);
}

#[inline]
pub(super) fn push_keyboard_byte(byte: u8, pressed: bool) {
    // SAFETY: hook is `fn`, install is single-threaded, read is value-copy.
    unsafe {
        if let Some(h) = KEYBOARD_SINK {
            h(byte, pressed);
        }
    }
}

#[inline]
pub(super) fn push_mouse(dx: i16, dy: i16, buttons: u8, wheel: i8) {
    unsafe {
        if let Some(h) = MOUSE_SINK {
            h(dx, dy, buttons, wheel);
        }
    }
}
