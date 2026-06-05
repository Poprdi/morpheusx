//! Kernel-side log helpers. Thin forwarders onto `morpheus-console` so kernel
//! log lines share the ONE console lock (no second serial path, no interleave).

use morpheus_console as console;

#[inline]
pub fn putc(b: u8) {
    console::putc(b);
}

#[inline]
pub fn puts(s: &str) {
    console::puts(s);
}

#[inline]
pub fn newline() {
    console::newline();
}

#[inline]
pub fn put_hex32(v: u32) {
    console::put_hex32(v);
}

#[inline]
pub fn put_hex64(v: u64) {
    console::put_hex64(v);
}

#[inline]
pub fn put_hex(v: u64) {
    console::put_hex64(v);
}

/// One atomic line: `[TAG] component <code>: msg\n` (single lock acquisition).
fn log_with_tag(tag: &str, component: &str, code: u16, msg: &str) {
    console::line(|w| {
        w.str("[");
        w.str(tag);
        w.str("] ");
        w.str(component);
        w.str(" ");
        w.dec_u16(code);
        w.str(": ");
        w.str(msg);
        w.str("\n");
    });
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
    console::checkpoint(label);
}

#[inline]
pub fn set_checkpoints_enabled(enabled: bool) {
    console::set_checkpoints_enabled(enabled);
}

/// Boot-log ring (now the real console ring, not a stub).
#[inline]
pub fn boot_log() -> &'static str {
    console::boot_log()
}

/// FB-only transient output.
#[inline]
pub fn fb_puts(s: &str) {
    console::fb_puts(s);
}
