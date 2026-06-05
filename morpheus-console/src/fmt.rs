//! Hex/decimal formatting helpers: the `puts_*` variants route through all sinks
//! via `puts`; the `put_hex*` variants emit a `0x`-prefixed value to the UART
//! under the line lock.

use crate::lock::LineGuard;
use crate::writer::{putc_raw, puts};

/// Low `n` hex nibbles, MSB-first, into `buf[..n]`.
#[inline]
fn fmt_hex(val: u64, n: usize, buf: &mut [u8]) {
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

/// UART hex, `0x` prefix, `n` digits.
#[inline]
fn put_hex_raw(val: u64, n: usize) {
    let mut buf = [0u8; 16];
    fmt_hex(val, n, &mut buf);
    let _guard = LineGuard::acquire();
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
