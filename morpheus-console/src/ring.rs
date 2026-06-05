//! The boot-log ring: a 64 KiB `'static` buffer that captures un-prefixed line
//! content up-front, reserved lock-free via CAS on the length.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

const BOOT_LOG_SIZE: usize = 64 * 1024;

struct LogBuf(UnsafeCell<[u8; BOOT_LOG_SIZE]>);

// SAFETY: writers reserve non-overlapping ranges via CAS on BOOT_LOG_LEN.
unsafe impl Sync for LogBuf {}

static BOOT_LOG_BUF: LogBuf = LogBuf(UnsafeCell::new([0u8; BOOT_LOG_SIZE]));
static BOOT_LOG_LEN: AtomicUsize = AtomicUsize::new(0);

#[inline]
pub(crate) fn log_capture(s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return;
    }

    loop {
        let current = BOOT_LOG_LEN.load(Ordering::Relaxed);
        let remaining = BOOT_LOG_SIZE.saturating_sub(current);
        let to_write = len.min(remaining);
        if to_write == 0 {
            return;
        }

        match BOOT_LOG_LEN.compare_exchange_weak(
            current,
            current + to_write,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                // SAFETY: this CAS reserved [current, current+to_write) exclusively.
                unsafe {
                    let buf = &mut *BOOT_LOG_BUF.0.get();
                    buf[current..current + to_write].copy_from_slice(&bytes[..to_write]);
                }
                return;
            },
            Err(_) => continue,
        }
    }
}

/// `'static` UTF-8 view of the boot-log buffer; never freed.
pub fn boot_log() -> &'static str {
    let len = BOOT_LOG_LEN.load(Ordering::Acquire);
    // SAFETY: bytes in [0, len) were committed by log_capture and never mutated
    // again; the buffer is `'static`.
    unsafe {
        let buf = &*BOOT_LOG_BUF.0.get();
        core::str::from_utf8(&buf[..len]).unwrap_or("")
    }
}
