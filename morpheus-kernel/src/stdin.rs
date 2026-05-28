//! Lock-free SPSC ring buffer for keyboard input. Producer: desktop event loop.
//! Consumer: `SYS_READ(fd=0)`. Head/tail acquire/release ordering replaces locks.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// Must be a power of two for the mask.
const BUF_SIZE: usize = 256;
const BUF_MASK: usize = BUF_SIZE - 1;

static mut BUF: [u8; BUF_SIZE] = [0; BUF_SIZE];
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);

/// Returns false if full (byte dropped). Single-producer only.
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

/// Returns bytes read (0 if empty). Single-consumer only.
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

pub fn available() -> usize {
    let head = HEAD.load(Ordering::Acquire);
    let tail = TAIL.load(Ordering::Relaxed);
    (head.wrapping_sub(tail)) & BUF_MASK
}

/// PID that receives Ctrl+C as SIGINT; 0 = none, Ctrl+C pushed to stdin instead.
static FOREGROUND_PID: AtomicU32 = AtomicU32::new(0);

pub fn set_foreground_pid(pid: u32) {
    FOREGROUND_PID.store(pid, Ordering::Release);
}

pub fn foreground_pid() -> u32 {
    FOREGROUND_PID.load(Ordering::Acquire)
}
