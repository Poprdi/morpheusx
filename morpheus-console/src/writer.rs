//! Byte/line output: the CRLF-translating `putc_raw`, the per-line timestamp +
//! cpu prefix, the `LineWriter` building block, and the locking line primitives
//! (`line`, `puts`, `putc`, `newline`).

use core::sync::atomic::{AtomicBool, Ordering};

use crate::lock::LineGuard;
use crate::ring::log_capture;
use crate::sink::{byte_sink, clock_us, cpu_id, fb_sink};

/// Set when the next UART byte begins a fresh line, so `putc_raw` emits the
/// `[<uptime>] [c<cpu>] ` prefix first. Toggled only under the line lock.
static AT_LINE_START: AtomicBool = AtomicBool::new(true);

/// Emit `[<secs>.<6-digit-us>] [c<cpu>] ` to the UART byte-sink only. Not the FB
/// (boot screen stays clean) and not the ring: the ring captures content up-front
/// in `LineWriter::str`, so a prefix appended here would land out of order — the
/// timestamp is a serial-wire artifact. Stamped at write time. Contains no `\n`,
/// so not CRLF-translated. Called from `putc_raw` with the line lock held.
#[inline]
fn emit_line_prefix() {
    #[inline]
    fn emit(b: u8) {
        if let Some(f) = byte_sink() {
            f(b);
        }
    }
    let mut buf = [0u8; 32];
    let mut n = 0usize;
    #[inline]
    fn push(buf: &mut [u8], n: &mut usize, b: u8) {
        if *n < buf.len() {
            buf[*n] = b;
            *n += 1;
        }
    }

    let us = clock_us();
    let secs = us / 1_000_000;
    let frac = us % 1_000_000;

    push(&mut buf, &mut n, b'[');
    // Seconds, right-justified to width 5.
    {
        let mut digits = [0u8; 20];
        let mut d = 0usize;
        let mut v = secs;
        if v == 0 {
            digits[0] = b'0';
            d = 1;
        } else {
            while v > 0 {
                digits[d] = b'0' + (v % 10) as u8;
                v /= 10;
                d += 1;
            }
        }
        let pad = 5usize.saturating_sub(d);
        for _ in 0..pad {
            push(&mut buf, &mut n, b' ');
        }
        while d > 0 {
            d -= 1;
            push(&mut buf, &mut n, digits[d]);
        }
    }
    push(&mut buf, &mut n, b'.');
    // Microseconds, zero-padded to 6 digits.
    {
        let mut div = 100_000u32;
        let mut f = frac as u32;
        while div > 0 {
            push(&mut buf, &mut n, b'0' + (f / div) as u8);
            f %= div;
            div /= 10;
        }
    }
    push(&mut buf, &mut n, b']');
    push(&mut buf, &mut n, b' ');
    push(&mut buf, &mut n, b'[');
    push(&mut buf, &mut n, b'c');
    {
        let cpu = cpu_id();
        let mut digits = [0u8; 10];
        let mut d = 0usize;
        let mut v = cpu;
        if v == 0 {
            digits[0] = b'0';
            d = 1;
        } else {
            while v > 0 {
                digits[d] = b'0' + (v % 10) as u8;
                v /= 10;
                d += 1;
            }
        }
        while d > 0 {
            d -= 1;
            push(&mut buf, &mut n, digits[d]);
        }
    }
    push(&mut buf, &mut n, b']');
    push(&mut buf, &mut n, b' ');

    for &b in &buf[..n] {
        emit(b);
    }
}

/// Byte to the UART sink, translating LF -> CRLF: raw serial terminals treat
/// `\n` as line-feed only, so without the CR every line staircases. CRLF lives
/// here only (the FB mirror handles bare `\n`). Before a new line's first byte we
/// stamp the prefix; a `\n` re-arms it for the next line.
#[inline]
pub(crate) fn putc_raw(b: u8) {
    if b != b'\n' && AT_LINE_START.swap(false, Ordering::Relaxed) {
        emit_line_prefix();
    }
    if b == b'\n' {
        if let Some(f) = byte_sink() {
            f(b'\r');
        }
        AT_LINE_START.store(true, Ordering::Relaxed);
    }
    if let Some(f) = byte_sink() {
        f(b);
    }
}

#[inline]
pub fn putc(b: u8) {
    let _guard = LineGuard::acquire();
    putc_raw(b);
}

/// Writes to every sink (boot-log ring, FB console, UART) with NO locking —
/// only valid while the caller holds the line lock via [`line`].
pub struct LineWriter;

impl LineWriter {
    #[inline]
    pub fn str(&mut self, s: &str) {
        log_capture(s);
        put_str_raw(s);
    }

    /// Append `v` as decimal digits.
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

/// Emit one complete line atomically: holds the line lock across the whole
/// closure so output never interleaves across cores. THE global primitive —
/// every producer funnels through it. Build the whole line in one call; never
/// call a locking fn (`puts`/`putc`) inside the closure (lock is not reentrant).
pub fn line(f: impl FnOnce(&mut LineWriter)) {
    let _guard = LineGuard::acquire();
    f(&mut LineWriter);
}

/// UART + boot-log + (if installed) live FB console — one atomic line.
pub fn puts(s: &str) {
    line(|w| w.str(s));
}

#[inline]
fn put_str_raw(s: &str) {
    for b in s.bytes() {
        // SAFETY: fb_sink holds a sink installed via set_fb_sink, safe per its
        // contract to call with any byte.
        unsafe {
            if let Some(f) = fb_sink() {
                f(b);
            }
        }
        putc_raw(b);
    }
}

#[inline]
fn put_dec_u16_raw(mut val: u16) {
    // Mirror digits to the FB as well as UART (matching put_str_raw) — otherwise
    // log codes only reach serial, not the screen.
    #[inline]
    fn emit(b: u8) {
        // SAFETY: fb_sink holds a sink installed via set_fb_sink, safe to call
        // with any byte.
        unsafe {
            if let Some(f) = fb_sink() {
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
pub fn newline() {
    let _guard = LineGuard::acquire();
    putc_raw(b'\n');
}

/// FB only; bypasses UART + log. Transient UI (spinners).
pub fn fb_puts(s: &str) {
    // SAFETY: fb_sink holds a sink installed via set_fb_sink, safe per its
    // contract to call with any byte.
    unsafe {
        if let Some(f) = fb_sink() {
            for b in s.bytes() {
                f(b);
            }
        }
    }
}

pub fn fb_putc(b: u8) {
    // SAFETY: see fb_puts.
    unsafe {
        if let Some(f) = fb_sink() {
            f(b);
        }
    }
}
