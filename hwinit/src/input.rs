//! Unified Input Subsystem
//!
//! Provides a single interface for keyboard and mouse input regardless of
//! source (PS/2 or USB HID). Device drivers register themselves with this
//! layer during early boot, and the desktop compositor reads from the
//! unified API.
//!
//! # Boot Sequence Constraint
//! All input device initialization MUST complete before the scheduler is
//! enabled. This ensures deterministic input handling from the very first
//! userspace process.
//!
//! # Design
//!
//! ```text
//! Device Drivers          Input Layer          Userspace
//!     |                      |                    |
//! PS/2 Keyboard ──────────►  │                    │
//! PS/2 Mouse   ───────────► │ ── unified API ──► │
//! USB HID Kbd ─────────────► │                    │
//! USB HID Mouse ────────────► │                    │
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // During early boot (before scheduler):
//! input_init();
//!
//! // Drivers register their handlers
//! register_keyboard_handler(ps2_keyboard_handler);
//! register_mouse_handler(ps2_mouse_handler);
//!
//! // Desktop compositor reads from unified API
//! if let Some(key) = poll_keyboard() { ... }
//! let (dx, dy, buttons) = poll_mouse();
//! ```

use crate::sync::SpinLock;

/// Input event types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Keyboard key event (scan_code, pressed)
    Key(u8, bool),
    /// Mouse movement (dx, dy)
    Move(i16, i16),
    /// Mouse button (button_index, pressed)
    Button(usize, bool),
    /// Mouse wheel scroll (delta)
    Wheel(i8),
}

/// Keyboard handler function type
type KeyboardHandler = fn(scan_code: u8, pressed: bool);
/// Mouse handler function type  
type MouseHandler = fn(dx: i16, dy: i16, buttons: u8);

// INTERNAL STATE

static KEYBOARD_HANDLERS: SpinLock<Option<KeyboardHandler>> = SpinLock::new(None);
static MOUSE_HANDLERS: SpinLock<Option<MouseHandler>> = SpinLock::new(None);

static KEYBOARD_EVENTS: SpinLock<[Option<InputEvent>; 32]> = SpinLock::new(
    [None; 32]
);
static MOUSE_EVENTS: SpinLock<[Option<InputEvent>; 16]> = SpinLock::new(
    [None; 16]
);

static KEYBOARD_HEAD: SpinLock<usize> = SpinLock::new(0);
static KEYBOARD_TAIL: SpinLock<usize> = SpinLock::new(0);
static MOUSE_HEAD: SpinLock<usize> = SpinLock::new(0);
static MOUSE_TAIL: SpinLock<usize> = SpinLock::new(0);

static KEYBOARD_REGISTERED: SpinLock<bool> = SpinLock::new(false);
static MOUSE_REGISTERED: SpinLock<bool> = SpinLock::new(false);

// UNIFIED INPUT API

/// Initialize the unified input subsystem.
/// Must be called during early boot before any input devices register.
pub fn init() {
    // Reset all state
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

/// Register a keyboard input handler.
/// Only one handler can be registered (PS/2 or USB, not both).
/// Panics if a handler is already registered.
pub fn register_keyboard_handler(handler: KeyboardHandler) {
    let mut registered = KEYBOARD_REGISTERED.lock();
    if *registered {
        panic!("Duplicate keyboard handler registration");
    }
    let mut handlers = KEYBOARD_HANDLERS.lock();
    *handlers = Some(handler);
    *registered = true;
}

/// Register a mouse input handler.
/// Only one handler can be registered (PS/2 or USB, not both).
/// Panics if a handler is already registered.
pub fn register_mouse_handler(handler: MouseHandler) {
    let mut registered = MOUSE_REGISTERED.lock();
    if *registered {
        panic!("Duplicate mouse handler registration");
    }
    let mut handlers = MOUSE_HANDLERS.lock();
    *handlers = Some(handler);
    *registered = true;
}

/// Push a keyboard event into the unified queue.
/// Called by registered keyboard handlers.
fn push_keyboard_event(event: InputEvent) {
    const MASK: usize = 31;
    
    let mut head = KEYBOARD_HEAD.lock();
    let mut tail = KEYBOARD_TAIL.lock();
    let next = (*head + 1) & MASK;
    
    if next != *tail {
        let mut events = KEYBOARD_EVENTS.lock();
        events[*head] = Some(event);
        *head = next;
    }
}

fn push_mouse_event(event: InputEvent) {
    const MASK: usize = 15;
    
    let mut head = MOUSE_HEAD.lock();
    let mut tail = MOUSE_TAIL.lock();
    let next = (*head + 1) & MASK;
    
    if next != *tail {
        let mut events = MOUSE_EVENTS.lock();
        events[*head] = Some(event);
        *head = next;
    }
}

/// Internal: Push a keyboard event directly.
/// Used by drivers to inject events into the unified queue.
pub fn push_keyboard_event_internal(event: InputEvent) {
    const MASK: usize = 31;
    
    let mut head = KEYBOARD_HEAD.lock();
    let mut tail = KEYBOARD_TAIL.lock();
    let next = (*head + 1) & MASK;
    
    if next != *tail {
        let mut events = KEYBOARD_EVENTS.lock();
        events[*head] = Some(event);
        *head = next;
    }
}

/// Internal: Push a mouse event directly.
/// Used by drivers to inject events into the unified queue.
pub fn push_mouse_event_internal(event: InputEvent) {
    const MASK: usize = 15;
    
    let mut head = MOUSE_HEAD.lock();
    let mut tail = MOUSE_TAIL.lock();
    let next = (*head + 1) & MASK;
    
    if next != *tail {
        let mut events = MOUSE_EVENTS.lock();
        events[*head] = Some(event);
        *head = next;
    }
}

/// Poll the next keyboard event from the unified queue.
/// Returns None if no events are pending.
pub fn poll_keyboard() -> Option<InputEvent> {
    let mut head = KEYBOARD_HEAD.lock();
    let mut tail = KEYBOARD_TAIL.lock();
    
    if *head == *tail {
        return None;
    }
    
    let events = KEYBOARD_EVENTS.lock();
    let event = events[*tail].take();
    *tail = (*tail + 1) & 31;
    drop(events);
    drop(tail);
    drop(head);
    
    event
}

/// Poll the next mouse event from the unified queue.
/// Returns None if no events are pending.
pub fn poll_mouse() -> Option<InputEvent> {
    let mut head = MOUSE_HEAD.lock();
    let mut tail = MOUSE_TAIL.lock();
    
    if *head == *tail {
        return None;
    }
    
    let events = MOUSE_EVENTS.lock();
    let event = events[*tail].take();
    *tail = (*tail + 1) & 15;
    drop(events);
    drop(tail);
    drop(head);
    
    event
}

/// Drain all pending mouse events and return aggregated state.
/// Returns (total_dx, total_dy, buttons).
/// Buttons is a bitmask: bit 0 = left, bit 1 = right, bit 2 = middle.
pub fn drain_mouse() -> (i32, i32, u8) {
    let mut dx: i32 = 0;
    let mut dy: i32 = 0;
    let mut buttons: u8 = 0;
    
    loop {
        match poll_mouse() {
            Some(InputEvent::Move(dx_i, dy_i)) => {
                dx += dx_i as i32;
                dy += dy_i as i32;
            }
            Some(InputEvent::Button(idx, pressed)) => {
                if pressed {
                    buttons |= 1 << idx;
                } else {
                    buttons &= !(1 << idx);
                }
            }
            Some(InputEvent::Wheel(delta)) => {
                // Wheel events can be accumulated separately if needed
                // For now, include sign in dx for apps that read it that way
                if delta > 0 {
                    dx += delta as i32;
                } else {
                    dx += delta as i32;
                }
            }
            None => break,
            _ => {}
        }
    }
    
    (dx, dy, buttons)
}

/// Check if a keyboard device is registered.
pub fn has_keyboard() -> bool {
    KEYBOARD_REGISTERED.lock().clone()
}

/// Check if a mouse device is registered.
pub fn has_mouse() -> bool {
    MOUSE_REGISTERED.lock().clone()
}

// LEGACY COMPATIBILITY STUBS
// These allow existing PS/2 and stdin code to work without modification

/// Legacy PS/2 keyboard scan code handler.
/// Translates PS/2 scan codes to the unified input layer.
pub fn ps2_keyboard_handler(scan_code: u8, pressed: bool) {
    let handler = KEYBOARD_HANDLERS.lock();
    if let Some(h) = *handler {
        h(scan_code, pressed);
    }
}

/// Legacy PS/2 mouse handler.
/// Translates PS/2 mouse data to the unified input layer.
pub fn ps2_mouse_handler(dx: i16, dy: i16, buttons: u8) {
    let handler = MOUSE_HANDLERS.lock();
    if let Some(h) = *handler {
        h(dx, dy, buttons);
    }
}
"