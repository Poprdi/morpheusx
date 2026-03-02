//! Kernel mouse state — global accumulator for PS/2 mouse deltas.
//!
//! The desktop event loop (producer) calls `accumulate(dx, dy, buttons)`.
//! `SYS_MOUSE_READ` (consumer) atomically drains the accumulated state.

use core::sync::atomic::{AtomicI32, AtomicU8, Ordering};

static DX: AtomicI32 = AtomicI32::new(0);
static DY: AtomicI32 = AtomicI32::new(0);
static BUTTONS: AtomicU8 = AtomicU8::new(0);

/// Accumulate relative motion from a PS/2 mouse packet.
pub fn accumulate(dx: i16, dy: i16, buttons: u8) {
    DX.fetch_add(dx as i32, Ordering::Relaxed);
    DY.fetch_add(dy as i32, Ordering::Relaxed);
    BUTTONS.store(buttons, Ordering::Relaxed);
}

/// Atomically drain accumulated mouse state. Returns (dx, dy, buttons).
/// Resets dx/dy to zero after reading.
pub fn drain() -> (i32, i32, u8) {
    let dx = DX.swap(0, Ordering::Relaxed);
    let dy = DY.swap(0, Ordering::Relaxed);
    let buttons = BUTTONS.load(Ordering::Relaxed);
    (dx, dy, buttons)
}
