//! Serial debug output (COM1 @ 0x3F8)
//!
//! Minimal post-EBS serial for hwinit debugging.
//! No buffering, no interrupts, pure polling.

const COM1: u16 = 0x3F8;
const COM1_LSR: u16 = COM1 + 5;
const LSR_TX_EMPTY: u8 = 0x20;

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

/// Write string to COM1.
pub fn puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

/// Write u32 as hex (0x prefix).
pub fn put_hex32(val: u32) {
    puts("0x");
    for i in (0..8).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
        putc(c);
    }
}

/// Write u64 as hex.
pub fn put_hex64(val: u64) {
    puts("0x");
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
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
