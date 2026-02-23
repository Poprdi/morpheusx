//! Serial debug output (COM1 @ 0x3F8)
//!
//! Minimal post-EBS serial for hwinit debugging.
//! No buffering, no interrupts, pure polling.
//!
//! All output is also written to a static ring buffer so the desktop
//! can display the full boot log on-screen before launching the shell.

const COM1: u16 = 0x3F8;
const COM1_LSR: u16 = COM1 + 5;
const LSR_TX_EMPTY: u8 = 0x20;

// ── Boot log capture ─────────────────────────────────────────────────────────

const BOOT_LOG_SIZE: usize = 64 * 1024; // 64 KB — enough for the full platform init

static mut BOOT_LOG_BUF: [u8; BOOT_LOG_SIZE] = [0u8; BOOT_LOG_SIZE];
static mut BOOT_LOG_LEN: usize = 0;

#[inline]
fn log_capture(s: &str) {
    unsafe {
        let bytes = s.as_bytes();
        let remaining = BOOT_LOG_SIZE.saturating_sub(BOOT_LOG_LEN);
        let to_write = bytes.len().min(remaining);
        if to_write > 0 {
            BOOT_LOG_BUF[BOOT_LOG_LEN..BOOT_LOG_LEN + to_write].copy_from_slice(&bytes[..to_write]);
            BOOT_LOG_LEN += to_write;
        }
    }
}

/// Return everything written to serial since boot as a UTF-8 string slice.
///
/// The returned slice is valid for `'static` — it points into the boot log
/// buffer which is never freed or overwritten after boot completes.
pub fn boot_log() -> &'static str {
    unsafe { core::str::from_utf8(&BOOT_LOG_BUF[..BOOT_LOG_LEN]).unwrap_or("") }
}

/// Write byte to COM1. Bounded wait, gives up after ~100 spins.
#[inline]
pub fn putc(b: u8) {
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

/// Write string to COM1 and capture it in the boot log buffer.
/// If a live framebuffer console hook is registered, writes there too.
pub fn puts(s: &str) {
    log_capture(s);
    for b in s.bytes() {
        // Live framebuffer console — registered by the bootloader once the
        // framebuffer is available. Writes to screen in real-time.
        unsafe {
            if let Some(f) = LIVE_PUTC {
                f(b);
            }
        }
        putc(b);
    }
}

// ── Live framebuffer console hook ────────────────────────────────────────────

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
    putc(b);
}

/// Write a string to COM1 only — no log capture, no framebuffer.
pub fn serial_puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

/// Write u32 as hex (0x prefix).
pub fn put_hex32(val: u32) {
    puts("0x");
    for i in (0..8).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        let c = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        };
        putc(c);
    }
}

/// Write u64 as hex.
pub fn put_hex64(val: u64) {
    puts("0x");
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        let c = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        };
        putc(c);
    }
}

/// Write u8 as hex (no prefix).
pub fn put_hex8(val: u8) {
    let hi = (val >> 4) & 0xF;
    let lo = val & 0xF;
    putc(if hi < 10 { b'0' + hi } else { b'a' + hi - 10 });
    putc(if lo < 10 { b'0' + lo } else { b'a' + lo - 10 });
}

/// Newline.
#[inline]
pub fn newline() {
    putc(b'\n');
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
