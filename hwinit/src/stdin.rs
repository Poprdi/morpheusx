//! Kernel stdin buffer — lock-free SPSC ring buffer for keyboard input.
//!
//! The desktop event loop (producer) pushes ASCII bytes via [`push()`].
//! `SYS_READ(fd=0)` (consumer) drains bytes via [`read()`].
//!
//! # Thread safety
//!
//! Single-producer / single-consumer on a single core.  Atomic loads/stores
//! on head and tail provide acquire/release ordering without locks.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Capacity of the stdin ring buffer (must be a power of two for mask trick).
const BUF_SIZE: usize = 256;
const BUF_MASK: usize = BUF_SIZE - 1;

/// Ring buffer storage.
static mut BUF: [u8; BUF_SIZE] = [0; BUF_SIZE];

/// Write cursor (producer advances).
static HEAD: AtomicUsize = AtomicUsize::new(0);

/// Read cursor (consumer advances).
static TAIL: AtomicUsize = AtomicUsize::new(0);

/// Push a single byte into the stdin buffer.
///
/// Returns `true` on success, `false` if the buffer is full (byte is dropped).
///
/// # Safety
///
/// Must be called from a single producer context (the desktop event loop).
pub fn push(byte: u8) -> bool {
    let head = HEAD.load(Ordering::Relaxed);
    let next = (head + 1) & BUF_MASK;

    if next == TAIL.load(Ordering::Acquire) {
        return false; // full
    }

    unsafe {
        BUF[head] = byte;
    }
    HEAD.store(next, Ordering::Release);
    true
}

/// Read up to `buf.len()` bytes from the stdin buffer.
///
/// Returns the number of bytes actually read (0 if the buffer is empty).
///
/// # Safety
///
/// Must be called from a single consumer context (the syscall handler).
pub fn read(buf: &mut [u8]) -> usize {
    let mut count = 0;

    while count < buf.len() {
        let tail = TAIL.load(Ordering::Relaxed);
        let head = HEAD.load(Ordering::Acquire);

        if tail == head {
            break; // empty
        }

        buf[count] = unsafe { BUF[tail] };
        TAIL.store((tail + 1) & BUF_MASK, Ordering::Release);
        count += 1;
    }

    count
}

/// Returns the number of bytes available to read without blocking.
pub fn available() -> usize {
    let head = HEAD.load(Ordering::Acquire);
    let tail = TAIL.load(Ordering::Relaxed);
    (head.wrapping_sub(tail)) & BUF_MASK
}
