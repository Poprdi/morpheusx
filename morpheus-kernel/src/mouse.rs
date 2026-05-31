//! Global mouse delta accumulator. Producers: desktop loop (PS/2) + USB HID
//! sink. Consumer: SYS_MOUSE_READ.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};

static DX: AtomicI32 = AtomicI32::new(0);
static DY: AtomicI32 = AtomicI32::new(0);
static WHEEL: AtomicI32 = AtomicI32::new(0);
static BUTTONS: AtomicU8 = AtomicU8::new(0);
static BUTTON_EDGE_PENDING: AtomicBool = AtomicBool::new(false);
static BUTTON_EDGE_SAMPLE: AtomicU8 = AtomicU8::new(0);

pub fn accumulate(dx: i16, dy: i16, buttons: u8) {
    accumulate_full(dx, dy, buttons, 0);
}

pub fn accumulate_full(dx: i16, dy: i16, buttons: u8, wheel: i8) {
    DX.fetch_add(dx as i32, Ordering::Relaxed);
    DY.fetch_add(dy as i32, Ordering::Relaxed);
    if wheel != 0 {
        WHEEL.fetch_add(wheel as i32, Ordering::Relaxed);
    }

    // Latch first edge so a fast press+release between polls is not lost.
    let prev = BUTTONS.swap(buttons, Ordering::Relaxed);
    if buttons != prev && !BUTTON_EDGE_PENDING.swap(true, Ordering::AcqRel) {
        BUTTON_EDGE_SAMPLE.store(buttons, Ordering::Relaxed);
    }
}

/// Drain accumulated state as (dx, dy, buttons, wheel); clears dx/dy/wheel.
pub fn drain() -> (i32, i32, u8, i32) {
    let dx = DX.swap(0, Ordering::Relaxed);
    let dy = DY.swap(0, Ordering::Relaxed);
    let wheel = WHEEL.swap(0, Ordering::Relaxed);
    let buttons = if BUTTON_EDGE_PENDING.swap(false, Ordering::AcqRel) {
        BUTTON_EDGE_SAMPLE.load(Ordering::Relaxed)
    } else {
        BUTTONS.load(Ordering::Relaxed)
    };
    (dx, dy, buttons, wheel)
}
