//! Unified keyboard/mouse input layer. PS/2 and USB HID drivers register a
//! single handler each; the desktop drains the unified queues. All
//! registration must happen before the scheduler starts.
//!
//! HID-to-input bridging: the HAL HID driver calls `push_keyboard_event_internal`
//! / `push_mouse_event_internal` via the `KernelHooks::{keyboard_sink, mouse_sink}`
//! callbacks the bootloader installs before `HalImpl::init`. The kernel
//! exports the public sink functions below; the bootloader wires them as
//! function pointers when it builds the hooks struct.

use crate::sync::SpinLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Key(u8, bool),
    Move(i16, i16),
    Button(usize, bool),
    Wheel(i8),
}

type KeyboardHandler = fn(scan_code: u8, pressed: bool);
type MouseHandler = fn(dx: i16, dy: i16, buttons: u8);

static KEYBOARD_HANDLERS: SpinLock<Option<KeyboardHandler>> = SpinLock::new(None);
static MOUSE_HANDLERS: SpinLock<Option<MouseHandler>> = SpinLock::new(None);

static KEYBOARD_EVENTS: SpinLock<[Option<InputEvent>; 32]> = SpinLock::new([None; 32]);
static MOUSE_EVENTS: SpinLock<[Option<InputEvent>; 16]> = SpinLock::new([None; 16]);

static KEYBOARD_HEAD: SpinLock<usize> = SpinLock::new(0);
static KEYBOARD_TAIL: SpinLock<usize> = SpinLock::new(0);
static MOUSE_HEAD: SpinLock<usize> = SpinLock::new(0);
static MOUSE_TAIL: SpinLock<usize> = SpinLock::new(0);

static KEYBOARD_REGISTERED: SpinLock<bool> = SpinLock::new(false);
static MOUSE_REGISTERED: SpinLock<bool> = SpinLock::new(false);

/// Call once during early boot before any device registers.
///
/// HID-to-input wiring was previously done here by reaching directly into
/// `usb::hid::sink`; after the kernel/HAL split that's the bootloader's job:
/// it builds a `KernelHooks` struct with function-pointer sinks pointing at
/// [`hid_keyboard_sink`] / [`hid_mouse_sink`] (which are public for that
/// purpose) and hands the hooks to `HalImpl::init`.
pub fn init() {
    {
        let mut head = KEYBOARD_HEAD.lock();
        *head = 0;
    }
    {
        let mut tail = KEYBOARD_TAIL.lock();
        *tail = 0;
    }
    {
        let mut head = MOUSE_HEAD.lock();
        *head = 0;
    }
    {
        let mut tail = MOUSE_TAIL.lock();
        *tail = 0;
    }
    {
        let mut handlers = KEYBOARD_HANDLERS.lock();
        *handlers = None;
    }
    {
        let mut handlers = MOUSE_HANDLERS.lock();
        *handlers = None;
    }
}

/// Public hook target: the HAL HID keyboard driver calls this (through the
/// `KernelHooks::keyboard_sink` function pointer the bootloader installs).
pub fn hid_keyboard_sink(byte: u8, pressed: bool) {
    push_keyboard_event_internal(InputEvent::Key(byte, pressed));
}

/// Public hook target: the HAL HID mouse driver calls this (through the
/// `KernelHooks::mouse_sink` function pointer the bootloader installs).
pub fn hid_mouse_sink(dx: i16, dy: i16, buttons: u8, wheel: i8) {
    push_mouse_event_internal(InputEvent::Move(dx, dy));
    push_mouse_event_internal(InputEvent::Button(0, (buttons & 0x01) != 0));
    push_mouse_event_internal(InputEvent::Button(1, (buttons & 0x02) != 0));
    push_mouse_event_internal(InputEvent::Button(2, (buttons & 0x04) != 0));
    if wheel != 0 {
        push_mouse_event_internal(InputEvent::Wheel(wheel));
    }
}

/// Panics on duplicate registration; PS/2 and USB are mutually exclusive.
pub fn register_keyboard_handler(handler: KeyboardHandler) {
    let mut registered = KEYBOARD_REGISTERED.lock();
    if *registered {
        panic!("Duplicate keyboard handler registration");
    }
    let mut handlers = KEYBOARD_HANDLERS.lock();
    *handlers = Some(handler);
    *registered = true;
}

/// Panics on duplicate registration; PS/2 and USB are mutually exclusive.
pub fn register_mouse_handler(handler: MouseHandler) {
    let mut registered = MOUSE_REGISTERED.lock();
    if *registered {
        panic!("Duplicate mouse handler registration");
    }
    let mut handlers = MOUSE_HANDLERS.lock();
    *handlers = Some(handler);
    *registered = true;
}

/// Driver-side injection into the keyboard queue.
pub fn push_keyboard_event_internal(event: InputEvent) {
    const MASK: usize = 31;

    let mut head = KEYBOARD_HEAD.lock();
    let tail = KEYBOARD_TAIL.lock();
    let next = (*head + 1) & MASK;

    if next != *tail {
        let mut events = KEYBOARD_EVENTS.lock();
        events[*head] = Some(event);
        *head = next;
    }
}

/// Driver-side injection into the mouse queue.
pub fn push_mouse_event_internal(event: InputEvent) {
    const MASK: usize = 15;

    let mut head = MOUSE_HEAD.lock();
    let tail = MOUSE_TAIL.lock();
    let next = (*head + 1) & MASK;

    if next != *tail {
        let mut events = MOUSE_EVENTS.lock();
        events[*head] = Some(event);
        *head = next;
    }
}

pub fn poll_keyboard() -> Option<InputEvent> {
    let head = KEYBOARD_HEAD.lock();
    let mut tail = KEYBOARD_TAIL.lock();

    if *head == *tail {
        return None;
    }

    let mut events = KEYBOARD_EVENTS.lock();
    let event = events[*tail].take();
    *tail = (*tail + 1) & 31;
    drop(events);
    drop(tail);
    drop(head);

    event
}

pub fn poll_mouse() -> Option<InputEvent> {
    let head = MOUSE_HEAD.lock();
    let mut tail = MOUSE_TAIL.lock();

    if *head == *tail {
        return None;
    }

    let mut events = MOUSE_EVENTS.lock();
    let event = events[*tail].take();
    *tail = (*tail + 1) & 15;
    drop(events);
    drop(tail);
    drop(head);

    event
}

/// (dx, dy, buttons) — buttons is bit0=L, bit1=R, bit2=M.
pub fn drain_mouse() -> (i32, i32, u8) {
    let mut dx: i32 = 0;
    let mut dy: i32 = 0;
    let mut buttons: u8 = 0;

    loop {
        match poll_mouse() {
            Some(InputEvent::Move(dx_i, dy_i)) => {
                dx += dx_i as i32;
                dy += dy_i as i32;
            },
            Some(InputEvent::Button(idx, pressed)) => {
                if pressed {
                    buttons |= 1 << idx;
                } else {
                    buttons &= !(1 << idx);
                }
            },
            Some(InputEvent::Wheel(delta)) => {
                // Folded into dx; no dedicated wheel channel yet.
                dx += delta as i32;
            },
            None => break,
            _ => {},
        }
    }

    (dx, dy, buttons)
}

pub fn has_keyboard() -> bool {
    *KEYBOARD_REGISTERED.lock()
}

pub fn has_mouse() -> bool {
    *MOUSE_REGISTERED.lock()
}

// Legacy PS/2 thunks — kept so existing call sites compile unchanged.

pub fn ps2_keyboard_handler(scan_code: u8, pressed: bool) {
    let handler = KEYBOARD_HANDLERS.lock();
    if let Some(h) = *handler {
        h(scan_code, pressed);
    }
}

pub fn ps2_mouse_handler(dx: i16, dy: i16, buttons: u8) {
    let handler = MOUSE_HANDLERS.lock();
    if let Some(h) = *handler {
        h(dx, dy, buttons);
    }
}
