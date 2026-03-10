//! Kernel stdout capture ring buffer for user-space process output.
//!
//! `sys_write(fd=1/2)` pushes bytes here in addition to serial output.
//! The desktop event loop drains the buffer via [`drain()`] and feeds
//! captured text into the Shell widget for on-screen display.
//!
//! Same SPSC design as stdin.rs — single producer (syscall handler),
//! single consumer (desktop event loop).

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const BUF_SIZE: usize = 8192;
const BUF_MASK: usize = BUF_SIZE - 1;

static mut BUF: [u8; BUF_SIZE] = [0; BUF_SIZE];

static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);
static PUSH_LOCK: crate::sync::IsrSafeRawSpinLock = crate::sync::IsrSafeRawSpinLock::new();

/// Master enable flag — the desktop sets this once the WM is ready
/// to receive process output. Before that, stdout goes to serial only.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable stdout capture. Called by the desktop after WM initialisation.
pub fn enable() {
    ENABLED.store(true, Ordering::Release);
}

/// Push bytes into the stdout capture buffer.
/// Silently drops bytes if the buffer is full (never blocks).
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
            return; // full — drop remainder
        }
        unsafe {
            BUF[head] = b;
        }
        HEAD.store(next, Ordering::Release);
    }
    PUSH_LOCK.unlock();
}

/// Drain all available bytes into `out`, appending up to `limit` bytes.
/// Returns the number of bytes drained.
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

/// Returns the number of bytes available to drain.
pub fn available() -> usize {
    let head = HEAD.load(Ordering::Acquire);
    let tail = TAIL.load(Ordering::Relaxed);
    (head.wrapping_sub(tail)) & BUF_MASK
}
