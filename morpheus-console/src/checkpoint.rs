//! Lock-free UART checkpoint marker that survives spinlock/heap deadlock, the
//! gating toggle, and the transient UART-only `serial_putc`/`serial_puts` paths.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::lock::LineGuard;
use crate::sink::byte_sink;
use crate::writer::putc_raw;

static CHECKPOINTS_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_checkpoints_enabled(enabled: bool) {
    CHECKPOINTS_ENABLED.store(enabled, Ordering::Release);
}

/// UART only; no log, no FB. Transient serial (\r overwrites, spinner frames).
#[inline]
pub fn serial_putc(b: u8) {
    let _guard = LineGuard::acquire();
    putc_raw(b);
}

pub fn serial_puts(s: &str) {
    let _guard = LineGuard::acquire();
    for b in s.bytes() {
        putc_raw(b);
    }
}

/// Lock-free UART marker that survives spinlock/heap deadlock: `[CP] label\r\n`.
/// If this doesn't appear, the fault is below software. Lock-free in that it never
/// takes the line lock, but still routes through (and depends on) the byte-sink.
#[inline(never)]
pub fn checkpoint(label: &str) {
    if !CHECKPOINTS_ENABLED.load(Ordering::Acquire) {
        return;
    }
    #[inline(always)]
    fn emit(b: u8) {
        if let Some(f) = byte_sink() {
            f(b);
        }
    }
    for b in b"[CP] ".iter().copied() {
        emit(b);
    }
    for b in label.bytes() {
        emit(b);
    }
    emit(b'\r');
    emit(b'\n');
}
