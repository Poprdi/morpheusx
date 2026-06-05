//! COM1 polling output mirrored into a 64 KiB boot-log ring.
//! SMP-safe via CAS on the log index + SpinLock on the port.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::sync::SpinLock;

const COM1: u16 = 0x3F8;
const COM1_IER: u16 = COM1 + 1; // doubles as DLM (divisor high) when DLAB=1
const COM1_FCR: u16 = COM1 + 2;
const COM1_LCR: u16 = COM1 + 3;
const COM1_MCR: u16 = COM1 + 4;
const COM1_LSR: u16 = COM1 + 5;
const LSR_TX_EMPTY: u8 = 0x20;

const BOOT_LOG_SIZE: usize = 64 * 1024;

struct LogBuf(UnsafeCell<[u8; BOOT_LOG_SIZE]>);

// SAFETY: writers reserve non-overlapping ranges via CAS on BOOT_LOG_LEN.
unsafe impl Sync for LogBuf {}

static BOOT_LOG_BUF: LogBuf = LogBuf(UnsafeCell::new([0u8; BOOT_LOG_SIZE]));
static BOOT_LOG_LEN: AtomicUsize = AtomicUsize::new(0);
static CHECKPOINTS_ENABLED: AtomicBool = AtomicBool::new(false);
static SERIAL_INIT_DONE: AtomicBool = AtomicBool::new(false);
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

/// Writes a byte to COM1, translating LF -> CRLF. Raw serial terminals
/// (screen/minicom) treat `\n` as line-feed only (down, not return), so without
/// the CR every line staircases. The FB mirror gets the untranslated byte
/// (its console handles bare `\n`), so this lives in the COM1 writer only.
#[inline]
fn putc_raw(b: u8) {
    if b == b'\n' {
        putc_raw_one(b'\r');
    }
    putc_raw_one(b);
}

/// Bounded ~100-spin wait for THR-empty before the OUT. Caller holds SERIAL_LOCK
/// or accepts interleaving.
#[inline]
fn putc_raw_one(b: u8) {
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

#[inline]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nostack, preserves_flags),
    );
}

/// Program COM1 to a known 115200 8N1 polled config, independent of firmware.
/// The rest of this module only ever writes THR / reads LSR, so without this it
/// inherits whatever baud + line control UEFI happened to leave — undefined
/// across boards, which silently produces garbage or nothing on a real RS-232
/// link. Idempotent; BSP-only, single-threaded early boot.
///
/// # Safety
/// Touches the `0x3F8` I/O-port range; call once on the BSP before any serial
/// output (`checkpoint`/log).
pub unsafe fn serial_init() {
    if SERIAL_INIT_DONE.swap(true, Ordering::AcqRel) {
        return;
    }
    // Divisor = 115200 / 115200 = 1 (use 0x0C for 9600).
    outb(COM1_IER, 0x00); // interrupts off — we poll
    outb(COM1_LCR, 0x80); // DLAB=1: next two writes are the divisor latch
    outb(COM1, 0x01); // DLL (divisor low)
    outb(COM1_IER, 0x00); // DLM (divisor high)
    outb(COM1_LCR, 0x03); // DLAB=0, 8 data bits / no parity / 1 stop
    outb(COM1_FCR, 0xC7); // FIFO enable + clear RX/TX, 14-byte trigger
    outb(COM1_MCR, 0x0B); // DTR + RTS + OUT2 asserted
}

/// Writes to every sink (boot-log ring, FB console, COM1) with NO locking —
/// only valid while the caller holds `SERIAL_LOCK` via [`line`].
pub struct LineWriter;

impl LineWriter {
    /// Append a string to the in-progress line.
    #[inline]
    pub fn str(&mut self, s: &str) {
        log_capture(s);
        put_str_raw(s);
    }

    /// Append `v` as decimal digits to the in-progress line.
    #[inline]
    pub fn dec_u16(&mut self, v: u16) {
        let mut tmp = [0u8; 5];
        let mut n = v;
        let mut i = tmp.len();
        if v == 0 {
            log_capture("0");
        } else {
            while n > 0 {
                i -= 1;
                tmp[i] = b'0' + (n % 10) as u8;
                n /= 10;
            }
            if let Ok(s) = core::str::from_utf8(&tmp[i..]) {
                log_capture(s);
            }
        }
        put_dec_u16_raw(v);
    }
}

/// Emit one complete line atomically: holds `SERIAL_LOCK` once across the whole
/// closure, so output never interleaves across cores. This is THE global
/// primitive — `puts`, `log_*`, `boot_step_*`, and any future producer funnel
/// through it and inherit atomicity for free. Build the entire line inside a
/// single `line(...)` call; never call a locking fn (`puts`/`putc`) from inside
/// the closure (the lock is not reentrant).
pub fn line(f: impl FnOnce(&mut LineWriter)) {
    let _guard = SERIAL_LOCK.lock();
    f(&mut LineWriter);
}

/// COM1 + boot-log + (if installed) live FB console — one atomic line.
pub fn puts(s: &str) {
    line(|w| w.str(s));
}

/// Low `n` hex nibbles, MSB-first, into `buf[..n]`.
#[inline]
fn fmt_hex(val: u64, n: usize, buf: &mut [u8]) {
    // `i` drives both the buffer slot and the nibble shift amount
    #[allow(clippy::needless_range_loop)]
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
    // Mirror digits to the FB console (LIVE_PUTC) as well as COM1, matching
    // put_str_raw — otherwise log codes only ever reach serial, not the screen.
    #[inline]
    fn emit(b: u8) {
        unsafe {
            if let Some(f) = LIVE_PUTC {
                f(b);
            }
        }
        putc_raw(b);
    }
    let mut buf = [0u8; 5];
    let mut i = 0usize;
    if val == 0 {
        emit(b'0');
        return;
    }
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        i += 1;
        val /= 10;
    }
    while i > 0 {
        i -= 1;
        emit(buf[i]);
    }
}

#[inline]
fn log_level(color: &str, level: &str, component: &str, code: u16, msg: &str) {
    let ansi = LOG_ANSI_ENABLED.load(Ordering::Acquire);
    let codes = LOG_CODES_ENABLED.load(Ordering::Acquire);
    line(|w| {
        if ansi {
            w.str(color);
        }
        w.str("[");
        w.str(level);
        w.str("] [");
        w.str(component);
        if codes {
            w.str(":");
            w.dec_u16(code);
        }
        w.str("] ");
        w.str(msg);
        if ansi {
            w.str(ANSI_RESET);
        }
        w.str("\n");
    });
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

/// Boot-chain title banner. One blank line, indented title + version, blank
/// line. Cyan when ANSI is enabled.
pub fn boot_banner(title: &str, version: &str) {
    let ansi = LOG_ANSI_ENABLED.load(Ordering::Acquire);
    puts("\n  ");
    if ansi {
        puts(ANSI_CYAN);
    }
    puts(title);
    if ansi {
        puts(ANSI_RESET);
    }
    puts("  ");
    puts(version);
    puts("\n\n");
}

/// One checklist row: `  [TAG]  label`. `tag` is the 4-char status text
/// (`" OK "`, `"WARN"`, `"FAIL"`); `color` tints only the bracketed tag.
fn boot_step(color: &str, tag: &str, label: &str) {
    let ansi = LOG_ANSI_ENABLED.load(Ordering::Acquire);
    line(|w| {
        w.str("  [");
        if ansi {
            w.str(color);
        }
        w.str(tag);
        if ansi {
            w.str(ANSI_RESET);
        }
        w.str("]  ");
        w.str(label);
        w.str("\n");
    });
}

/// Happy-path checklist marker — green `[ OK ]`.
pub fn boot_step_ok(label: &str) {
    boot_step(ANSI_GREEN, " OK ", label);
}

/// Non-fatal checklist marker — yellow `[WARN]`. The detailed `log_warn`
/// lines that follow carry the specifics.
pub fn boot_step_warn(label: &str) {
    boot_step(ANSI_YELLOW, "WARN", label);
}

/// Fatal checklist marker — red `[FAIL]`.
pub fn boot_step_fail(label: &str) {
    boot_step(ANSI_RED, "FAIL", label);
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
