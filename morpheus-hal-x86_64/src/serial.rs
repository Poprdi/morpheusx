//! COM1 polling output mirrored into a 64 KiB boot-log ring.
//! SMP-safe via CAS on the log index + SpinLock on the port.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::sync::SpinLock;

const COM1: u16 = 0x3F8;
const COM1_LSR: u16 = COM1 + 5;
const LSR_TX_EMPTY: u8 = 0x20;

const BOOT_LOG_SIZE: usize = 64 * 1024;

struct LogBuf(UnsafeCell<[u8; BOOT_LOG_SIZE]>);

// SAFETY: writers reserve non-overlapping ranges via CAS on BOOT_LOG_LEN.
unsafe impl Sync for LogBuf {}

static BOOT_LOG_BUF: LogBuf = LogBuf(UnsafeCell::new([0u8; BOOT_LOG_SIZE]));
static BOOT_LOG_LEN: AtomicUsize = AtomicUsize::new(0);
static CHECKPOINTS_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_ANSI_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_CODES_ENABLED: AtomicBool = AtomicBool::new(false);

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_RED: &str = "\x1b[31m";

/// Serializes COM1 + FB-console hook across cores; SpinLock saves IF (ISR-safe).
static SERIAL_LOCK: SpinLock<()> = SpinLock::new(());

#[inline]
fn log_capture(s: &str) {
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
    unsafe {
        let buf = &*BOOT_LOG_BUF.0.get();
        core::str::from_utf8(&buf[..len]).unwrap_or("")
    }
}

/// Bounded ~100-spin wait for THR-empty before the OUT. Caller holds SERIAL_LOCK
/// or accepts interleaving.
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

#[inline]
pub fn putc(b: u8) {
    let _guard = SERIAL_LOCK.lock();
    putc_raw(b);
}

/// COM1 + boot-log + (if installed) live FB console.
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

/// Low `n` hex nibbles, MSB-first, into `buf[..n]`.
#[inline]
fn fmt_hex(val: u64, n: usize, buf: &mut [u8]) {
    for i in 0..n {
        let nyb = ((val >> ((n - 1 - i) * 4)) & 0xF) as u8;
        buf[i] = if nyb < 10 {
            b'0' + nyb
        } else {
            b'a' + (nyb - 10)
        };
    }
}

/// 8 hex digits, no `0x`, via `puts` (all sinks).
pub fn puts_hex_u32(val: u32) {
    let mut buf = [0u8; 8];
    fmt_hex(val as u64, 8, &mut buf);
    if let Ok(s) = core::str::from_utf8(&buf) {
        puts(s);
    }
}

pub fn puts_hex_u64(val: u64) {
    puts_hex_u32((val >> 32) as u32);
    puts_hex_u32(val as u32);
}

pub fn puts_hex_u8(val: u8) {
    let mut buf = [0u8; 2];
    fmt_hex(val as u64, 2, &mut buf);
    if let Ok(s) = core::str::from_utf8(&buf) {
        puts(s);
    }
}

/// Up to 10 decimal digits via `puts`.
pub fn puts_dec_u32(val: u32) {
    if val == 0 {
        puts("0");
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    let mut v = val;
    while v > 0 {
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    buf[..i].reverse();
    if let Ok(s) = core::str::from_utf8(&buf[..i]) {
        puts(s);
    }
}

pub fn puts_dec_u8(val: u8) {
    if val >= 100 {
        let mut buf = [b'0'; 3];
        buf[0] = b'0' + (val / 100);
        buf[1] = b'0' + ((val / 10) % 10);
        buf[2] = b'0' + (val % 10);
        if let Ok(s) = core::str::from_utf8(&buf) {
            puts(s);
        }
    } else if val >= 10 {
        let mut buf = [b'0'; 2];
        buf[0] = b'0' + (val / 10);
        buf[1] = b'0' + (val % 10);
        if let Ok(s) = core::str::from_utf8(&buf) {
            puts(s);
        }
    } else {
        let buf = [b'0' + val];
        if let Ok(s) = core::str::from_utf8(&buf) {
            puts(s);
        }
    }
}

#[inline]
fn put_str_raw(s: &str) {
    for b in s.bytes() {
        unsafe {
            if let Some(f) = LIVE_PUTC {
                f(b);
            }
        }
        putc_raw(b);
    }
}

#[inline]
fn put_dec_u16_raw(mut val: u16) {
    let mut buf = [0u8; 5];
    let mut i = 0usize;
    if val == 0 {
        putc_raw(b'0');
        return;
    }
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        i += 1;
        val /= 10;
    }
    while i > 0 {
        i -= 1;
        putc_raw(buf[i]);
    }
}

#[inline]
fn log_level(color: &str, level: &str, component: &str, code: u16, msg: &str) {
    let ansi = LOG_ANSI_ENABLED.load(Ordering::Acquire);
    let codes = LOG_CODES_ENABLED.load(Ordering::Acquire);

    // Boot-log capture mirrors the wire.
    if ansi {
        log_capture(color);
    }
    log_capture("[");
    log_capture(level);
    log_capture("] [");
    log_capture(component);
    if codes {
        log_capture(":");
        {
            let mut tmp = [0u8; 5];
            let mut n = code;
            let mut i = 0usize;
            if n == 0 {
                log_capture("0");
            } else {
                while n > 0 {
                    tmp[i] = b'0' + (n % 10) as u8;
                    i += 1;
                    n /= 10;
                }
                while i > 0 {
                    i -= 1;
                    let b = [tmp[i]];
                    if let Ok(s) = core::str::from_utf8(&b) {
                        log_capture(s);
                    }
                }
            }
        }
    }
    log_capture("] ");
    log_capture(msg);
    if ansi {
        log_capture(ANSI_RESET);
    }
    log_capture("\n");

    // One line under one lock — no SMP interleave.
    let _guard = SERIAL_LOCK.lock();
    if ansi {
        put_str_raw(color);
    }
    put_str_raw("[");
    put_str_raw(level);
    put_str_raw("] [");
    put_str_raw(component);
    if codes {
        put_str_raw(":");
        put_dec_u16_raw(code);
    }
    put_str_raw("] ");
    put_str_raw(msg);
    if ansi {
        put_str_raw(ANSI_RESET);
    }
    put_str_raw("\n");
}

pub fn log_info(component: &str, code: u16, msg: &str) {
    log_level(ANSI_CYAN, "INFO", component, code, msg);
}

pub fn log_ok(component: &str, code: u16, msg: &str) {
    log_level(ANSI_GREEN, "OK", component, code, msg);
}

pub fn log_warn(component: &str, code: u16, msg: &str) {
    log_level(ANSI_YELLOW, "WARN", component, code, msg);
}

pub fn log_error(component: &str, code: u16, msg: &str) {
    log_level(ANSI_RED, "ERR", component, code, msg);
}

pub fn set_checkpoints_enabled(enabled: bool) {
    CHECKPOINTS_ENABLED.store(enabled, Ordering::Release);
}

/// `ansi=false` strips escapes; `codes=false` hides event numbers.
pub fn set_log_style(ansi_enabled: bool, codes_enabled: bool) {
    LOG_ANSI_ENABLED.store(ansi_enabled, Ordering::Release);
    LOG_CODES_ENABLED.store(codes_enabled, Ordering::Release);
}

/// FB mirror installed by bootloader; `None` until the FB is up.
static mut LIVE_PUTC: Option<unsafe fn(u8)> = None;

pub fn set_live_console_hook(f: unsafe fn(u8)) {
    unsafe { LIVE_PUTC = Some(f) }
}

pub fn clear_live_console_hook() {
    unsafe { LIVE_PUTC = None }
}

/// FB only; bypasses COM1 + log. Transient UI (spinners).
pub fn fb_puts(s: &str) {
    unsafe {
        if let Some(f) = LIVE_PUTC {
            for b in s.bytes() {
                f(b);
            }
        }
    }
}

pub fn fb_putc(b: u8) {
    unsafe {
        if let Some(f) = LIVE_PUTC {
            f(b);
        }
    }
}

/// COM1 only; no log, no FB. Transient serial (\r overwrites, spinner frames).
#[inline]
pub fn serial_putc(b: u8) {
    let _guard = SERIAL_LOCK.lock();
    putc_raw(b);
}

pub fn serial_puts(s: &str) {
    let _guard = SERIAL_LOCK.lock();
    for b in s.bytes() {
        putc_raw(b);
    }
}

/// Lock-free COM1 marker that survives spinlock/heap deadlock.
/// `[CP] label\r\n`. If this doesn't appear, the fault is below software.
#[inline(never)]
pub fn checkpoint(label: &str) {
    if !CHECKPOINTS_ENABLED.load(Ordering::Acquire) {
        return;
    }
    #[inline(always)]
    fn emit(b: u8) {
        unsafe {
            // Bounded — don't hang if the UART itself is wedged.
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
    for b in b"[CP] ".iter().copied() {
        emit(b);
    }
    for b in label.bytes() {
        emit(b);
    }
    emit(b'\r');
    emit(b'\n');
}

/// COM1 hex, `0x` prefix, `n` digits.
#[inline]
fn put_hex_raw(val: u64, n: usize) {
    let mut buf = [0u8; 16];
    fmt_hex(val, n, &mut buf);
    let _guard = SERIAL_LOCK.lock();
    putc_raw(b'0');
    putc_raw(b'x');
    for &b in &buf[..n] {
        putc_raw(b);
    }
}

pub fn put_hex32(val: u32) {
    put_hex_raw(val as u64, 8);
}

pub fn put_hex64(val: u64) {
    put_hex_raw(val, 16);
}

pub fn put_hex8(val: u8) {
    put_hex_raw(val as u64, 2);
}

#[inline]
pub fn newline() {
    let _guard = SERIAL_LOCK.lock();
    putc_raw(b'\n');
}
