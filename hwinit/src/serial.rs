//! Serial debug output (COM1 @ 0x3F8)
//!
//! Minimal post-EBS serial for hwinit debugging.
//! No buffering, no interrupts, pure polling.
//!
//! All output is also written to a static ring buffer so the desktop
//! can display the full boot log on-screen before launching the shell.
//!
//! SMP-safe: the log buffer uses atomic indexing, serial port access
//! is serialized by a spinlock.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::sync::SpinLock;

const COM1: u16 = 0x3F8;
const COM1_LSR: u16 = COM1 + 5;
const LSR_TX_EMPTY: u8 = 0x20;

// boot log capture — SMP-safe via atomic CAS on the write index

const BOOT_LOG_SIZE: usize = 64 * 1024; // 64 KB — enough for the full platform init

/// Buffer wrapper. UnsafeCell because multiple cores write to non-overlapping
/// slices (guaranteed by the atomic CAS reservation below).
struct LogBuf(UnsafeCell<[u8; BOOT_LOG_SIZE]>);

// non-overlapping writes to distinct indices are safe
unsafe impl Sync for LogBuf {}

static BOOT_LOG_BUF: LogBuf = LogBuf(UnsafeCell::new([0u8; BOOT_LOG_SIZE]));
static BOOT_LOG_LEN: AtomicUsize = AtomicUsize::new(0);

/// Serializes COM1 port access + framebuffer console hook across cores.
/// SpinLock saves/disables/restores IF — safe from ISR context.
static SERIAL_LOCK: SpinLock<()> = SpinLock::new(());

#[inline]
fn log_capture(s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return;
    }

    // CAS loop to atomically reserve a contiguous range
    loop {
        let current = BOOT_LOG_LEN.load(Ordering::Relaxed);
        let remaining = BOOT_LOG_SIZE.saturating_sub(current);
        let to_write = len.min(remaining);
        if to_write == 0 {
            return; // buffer full
        }

        match BOOT_LOG_LEN.compare_exchange_weak(
            current,
            current + to_write,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                // we own [current..current+to_write] exclusively
                unsafe {
                    let buf = &mut *BOOT_LOG_BUF.0.get();
                    buf[current..current + to_write].copy_from_slice(&bytes[..to_write]);
                }
                return;
            }
            Err(_) => continue, // another core won the race, try again
        }
    }
}

/// Return everything written to serial since boot as a UTF-8 string slice.
///
/// The returned slice is valid for `'static` — it points into the boot log
/// buffer which is never freed or overwritten after boot completes.
pub fn boot_log() -> &'static str {
    let len = BOOT_LOG_LEN.load(Ordering::Acquire);
    unsafe {
        let buf = &*BOOT_LOG_BUF.0.get();
        core::str::from_utf8(&buf[..len]).unwrap_or("")
    }
}

/// Write byte to COM1. Bounded wait, gives up after ~100 spins.
/// Does NOT acquire the serial lock — callers must hold it or accept interleaving.
#[inline]
fn putc_raw(b: u8) {
    unsafe {
        for _ in 0..100 {
            let status: u8;
            core::arch::asm!(
                "in al, dx",
                in("dx") COM1_LSR,
                out("al") status,
                options(nostack, preserves_flags)
            );
            if status & LSR_TX_EMPTY != 0 {
                core::arch::asm!(
                    "out dx, al",
                    in("dx") COM1,
                    in("al") b,
                    options(nostack, preserves_flags)
                );
                return;
            }
            core::hint::spin_loop();
        }
    }
}

/// Write byte to COM1 under the serial lock.
#[inline]
pub fn putc(b: u8) {
    let _guard = SERIAL_LOCK.lock();
    putc_raw(b);
}

/// Write string to COM1 and capture it in the boot log buffer.
/// If a live framebuffer console hook is registered, writes there too.
/// Serialized across cores — no byte interleaving.
pub fn puts(s: &str) {
    log_capture(s);
    let _guard = SERIAL_LOCK.lock();
    for b in s.bytes() {
        unsafe {
            if let Some(f) = LIVE_PUTC {
                f(b);
            }
        }
        putc_raw(b);
    }
}

// live framebuffer console hook

/// Function pointer set by the bootloader to mirror serial output to the
/// framebuffer. `None` until the framebuffer is ready.
static mut LIVE_PUTC: Option<unsafe fn(u8)> = None;

/// Register a live framebuffer putc hook. Called by the bootloader once the
/// framebuffer is available. After this every `puts()` call also writes to
/// the screen in real-time.
pub fn set_live_console_hook(f: unsafe fn(u8)) {
    unsafe { LIVE_PUTC = Some(f) }
}

/// Unregister the live hook. Called before the window manager takes over.
pub fn clear_live_console_hook() {
    unsafe { LIVE_PUTC = None }
}

/// Write to the live framebuffer console only — does NOT go to COM1 or the
/// boot log buffer. Use this for transient visual feedback (spinners, etc.)
/// that should not pollute the serial log.
pub fn fb_puts(s: &str) {
    unsafe {
        if let Some(f) = LIVE_PUTC {
            for b in s.bytes() {
                f(b);
            }
        }
    }
}

/// Same as `fb_puts` but for a single byte.
pub fn fb_putc(b: u8) {
    unsafe {
        if let Some(f) = LIVE_PUTC {
            f(b);
        }
    }
}

/// Write a byte to COM1 only — no log capture, no framebuffer.
/// Use this for transient output (spinner frames, \r overwrites) that must
/// appear on serial without going into the boot log buffer.
#[inline]
pub fn serial_putc(b: u8) {
    let _guard = SERIAL_LOCK.lock();
    putc_raw(b);
}

/// Write a string to COM1 only — no log capture, no framebuffer.
pub fn serial_puts(s: &str) {
    let _guard = SERIAL_LOCK.lock();
    for b in s.bytes() {
        putc_raw(b);
    }
}

/// Write a checkpoint marker directly to COM1 with zero locking.
///
/// Black-box debug tool — works even if SERIAL_LOCK, GLOBAL_REGISTRY,
/// PROCESS_TABLE_LOCK, or the heap spinlock is deadlocked.
/// Format: "[CP] label\r\n".  No log capture.  No lock.  No `unsafe` required.
/// If this doesn't appear on serial, the problem is below software.
#[inline(never)]
pub fn checkpoint(label: &str) {
    #[inline(always)]
    fn emit(b: u8) {
        unsafe {
            // bounded spin so we don't hang here if UART itself is wedged
            for _ in 0..100_000u32 {
                let status: u8;
                core::arch::asm!(
                    "in al, dx",
                    in("dx") COM1_LSR,
                    out("al") status,
                    options(nostack, preserves_flags)
                );
                if status & LSR_TX_EMPTY != 0 {
                    core::arch::asm!(
                        "out dx, al",
                        in("dx") COM1,
                        in("al") b,
                        options(nostack, preserves_flags)
                    );
                    return;
                }
                core::hint::spin_loop();
            }
        }
    }
    for b in b"[CP] ".iter().copied() { emit(b); }
    for b in label.bytes() { emit(b); }
    emit(b'\r');
    emit(b'\n');
}

/// Write u32 as hex (0x prefix).
pub fn put_hex32(val: u32) {
    let _guard = SERIAL_LOCK.lock();
    putc_raw(b'0');
    putc_raw(b'x');
    for i in (0..8).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        let c = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        };
        putc_raw(c);
    }
}

/// Write u64 as hex.
pub fn put_hex64(val: u64) {
    let _guard = SERIAL_LOCK.lock();
    putc_raw(b'0');
    putc_raw(b'x');
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        let c = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        };
        putc_raw(c);
    }
}

/// Write u8 as hex (no prefix).
pub fn put_hex8(val: u8) {
    let _guard = SERIAL_LOCK.lock();
    let hi = (val >> 4) & 0xF;
    let lo = val & 0xF;
    putc_raw(if hi < 10 { b'0' + hi } else { b'a' + hi - 10 });
    putc_raw(if lo < 10 { b'0' + lo } else { b'a' + lo - 10 });
}

/// Newline.
#[inline]
pub fn newline() {
    let _guard = SERIAL_LOCK.lock();
    putc_raw(b'\n');
}

/// Debug log with [HWINIT] prefix.
#[macro_export]
macro_rules! dbg {
    ($($arg:tt)*) => {{
        $crate::serial::puts("[HWINIT] ");
        $crate::serial::puts($($arg)*);
        $crate::serial::newline();
    }};
}

/// Debug log with hex value.
#[macro_export]
macro_rules! dbg_hex {
    ($msg:expr, $val:expr) => {{
        $crate::serial::puts("[HWINIT] ");
        $crate::serial::puts($msg);
        $crate::serial::put_hex32($val as u32);
        $crate::serial::newline();
    }};
}
