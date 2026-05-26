//! Kernel-side log helpers built on top of the HAL `Serial` primitives.

use crate::hal;
use core::sync::atomic::{AtomicBool, Ordering};

static CHECKPOINTS_ENABLED: AtomicBool = AtomicBool::new(true);

#[inline]
pub fn putc(b: u8) {
    hal().serial().putc(b);
}

#[inline]
pub fn puts(s: &str) {
    hal().serial().puts(s);
}

#[inline]
pub fn newline() {
    hal().serial().newline();
}

#[inline]
pub fn put_hex32(v: u32) {
    hal().serial().put_hex32(v);
}

#[inline]
pub fn put_hex64(v: u64) {
    hal().serial().put_hex64(v);
}

#[inline]
pub fn put_hex(v: u64) {
    hal().serial().put_hex64(v);
}

fn put_dec_u16(v: u16) {
    let mut buf = [0u8; 5];
    let mut n = v;
    let mut i = buf.len();
    if n == 0 {
        putc(b'0');
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    for &b in &buf[i..] {
        putc(b);
    }
}

fn log_with_tag(tag: &str, component: &str, code: u16, msg: &str) {
    puts("[");
    puts(tag);
    puts("] ");
    puts(component);
    puts(" ");
    put_dec_u16(code);
    puts(": ");
    puts(msg);
    newline();
}

#[inline]
pub fn log_info(component: &str, code: u16, msg: &str) {
    log_with_tag("INFO", component, code, msg);
}

#[inline]
pub fn log_ok(component: &str, code: u16, msg: &str) {
    log_with_tag("OK", component, code, msg);
}

#[inline]
pub fn log_warn(component: &str, code: u16, msg: &str) {
    log_with_tag("WARN", component, code, msg);
}

#[inline]
pub fn log_error(component: &str, code: u16, msg: &str) {
    log_with_tag("ERR", component, code, msg);
}

#[inline]
pub fn checkpoint(label: &str) {
    if !CHECKPOINTS_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    puts("CHK ");
    puts(label);
    newline();
}

#[inline]
pub fn set_checkpoints_enabled(enabled: bool) {
    CHECKPOINTS_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Stub: ring lives in HAL; trait lacks an accessor.
#[inline]
pub fn boot_log() -> &'static str {
    ""
}

/// Stub: FB console still owned bootloader-side.
#[inline]
pub fn fb_puts(_s: &str) {}
