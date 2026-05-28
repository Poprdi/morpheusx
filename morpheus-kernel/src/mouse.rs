//! Global PS/2 mouse delta accumulator. Producer: desktop loop. Consumer: SYS_MOUSE_READ.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};

static DX: AtomicI32 = AtomicI32::new(0);
static DY: AtomicI32 = AtomicI32::new(0);
static BUTTONS: AtomicU8 = AtomicU8::new(0);
static BUTTON_EDGE_PENDING: AtomicBool = AtomicBool::new(false);
static BUTTON_EDGE_SAMPLE: AtomicU8 = AtomicU8::new(0);

pub fn accumulate(dx: i16, dy: i16, buttons: u8) {
    DX.fetch_add(dx as i32, Ordering::Relaxed);
    DY.fetch_add(dy as i32, Ordering::Relaxed);

    // Latch first edge so fast press+release between polls is not lost.
    let prev = BUTTONS.swap(buttons, Ordering::Relaxed);
    if buttons != prev && !BUTTON_EDGE_PENDING.swap(true, Ordering::AcqRel) {
        BUTTON_EDGE_SAMPLE.store(buttons, Ordering::Relaxed);
    }
}

/// Drain accumulated state as (dx, dy, buttons); clears dx/dy.
pub fn drain() -> (i32, i32, u8) {
    let dx = DX.swap(0, Ordering::Relaxed);
    let dy = DY.swap(0, Ordering::Relaxed);
    let buttons = if BUTTON_EDGE_PENDING.swap(false, Ordering::AcqRel) {
        BUTTON_EDGE_SAMPLE.load(Ordering::Relaxed)
    } else {
        BUTTONS.load(Ordering::Relaxed)
    };
    (dx, dy, buttons)
}
