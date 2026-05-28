//! SPSC ring buffer mirroring user stdout (fd=1/2) to the desktop Shell widget.
//! Producer: sys_write. Consumer: desktop event loop.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const BUF_SIZE: usize = 8192;
const BUF_MASK: usize = BUF_SIZE - 1;

static mut BUF: [u8; BUF_SIZE] = [0; BUF_SIZE];

static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);
static PUSH_LOCK: crate::sync::IsrSafeRawSpinLock = crate::sync::IsrSafeRawSpinLock::new();

/// Set once the WM is ready; until then stdout is serial-only.
static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn enable() {
    ENABLED.store(true, Ordering::Release);
}

/// Drops bytes if full, never blocks.
pub fn push(data: &[u8]) {
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    PUSH_LOCK.lock();
    for &b in data {
        let head = HEAD.load(Ordering::Relaxed);
        let next = (head + 1) & BUF_MASK;
        if next == TAIL.load(Ordering::Acquire) {
            PUSH_LOCK.unlock();
            return;
        }
        unsafe {
            BUF[head] = b;
        }
        HEAD.store(next, Ordering::Release);
    }
    PUSH_LOCK.unlock();
}

/// Drain up to `out.len()` bytes; returns count drained.
pub fn drain(out: &mut [u8]) -> usize {
    let mut count = 0;
    while count < out.len() {
        let tail = TAIL.load(Ordering::Relaxed);
        let head = HEAD.load(Ordering::Acquire);
        if tail == head {
            break;
        }
        out[count] = unsafe { BUF[tail] };
        TAIL.store((tail + 1) & BUF_MASK, Ordering::Release);
        count += 1;
    }
    count
}

pub fn available() -> usize {
    let head = HEAD.load(Ordering::Acquire);
    let tail = TAIL.load(Ordering::Relaxed);
    (head.wrapping_sub(tail)) & BUF_MASK
}
