//! x86-only serial seam for the portable `morpheus-console`.
//!
//! All platform-agnostic logic (boot-log ring, per-line lock, CRLF, ANSI/log
//! levels, `boot_step*`, format helpers, checkpoint) now lives in
//! `morpheus-console` and is re-exported below so every existing
//! `morpheus_hal_x86_64::serial::{...}` path resolves unchanged.
//!
//! This module keeps ONLY the x86 bits: programming the 16550 (`serial_init`),
//! the raw byte-out to port `0x3F8` (with a bounded LSR poll so a wedged UART
//! can't deadlock logging), and IRQ save/restore (`pushf;cli` / `popf`). The
//! latter two are installed into the console as hooks by `serial_init`.

use core::sync::atomic::{AtomicBool, Ordering};

// === Re-exports: the portable console surface ===============================
pub use morpheus_console::{
    boot_banner, boot_log, boot_step_fail, boot_step_ok, boot_step_warn, checkpoint,
    clear_live_console_hook, fb_putc, fb_puts, line, log_error, log_info, log_ok, log_warn,
    newline, put_hex32, put_hex64, put_hex8, putc, puts, puts_dec_u32, puts_dec_u8, puts_hex_u32,
    puts_hex_u64, puts_hex_u8, serial_putc, serial_puts, set_checkpoints_enabled,
    set_live_console_hook, set_log_style, LineWriter,
};

// === x86 port definitions ===================================================

const COM1: u16 = 0x3F8;
const COM1_IER: u16 = COM1 + 1; // doubles as DLM (divisor high) when DLAB=1
const COM1_FCR: u16 = COM1 + 2;
const COM1_LCR: u16 = COM1 + 3;
const COM1_MCR: u16 = COM1 + 4;
const COM1_LSR: u16 = COM1 + 5;
const LSR_TX_EMPTY: u8 = 0x20;
const LSR_RX_READY: u8 = 0x01;

static SERIAL_INIT_DONE: AtomicBool = AtomicBool::new(false);

#[inline]
unsafe fn outb(port: u16, val: u8) {
    // SAFETY: caller targets a known 16550 register in the 0x3F8 range; a port
    // OUT has no memory effects.
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nostack, preserves_flags),
    );
}

/// Console byte-sink: bounded ~100-spin wait for THR-empty, then OUT one byte.
///
/// Registered with `morpheus_console::set_byte_sink`. The console already
/// inserts `\r` before `\n`, so this does NO CRLF translation — it just polls
/// then writes the single byte. The bounded poll guarantees `checkpoint`/logging
/// can't hang on a wedged UART (it drops the byte after the cap instead).
fn raw_byte_out(b: u8) {
    // SAFETY: reads LSR and writes THR on COM1; ports have no memory effects.
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

/// Non-blocking read of one byte from the COM1 receive register, or `None` if the RX
/// FIFO is empty.
///
/// We deliberately poll rather than enable the RX-available IRQ (IER stays 0): the
/// bootloader's input-forwarding loop already runs every tick and drains this beside
/// the PS/2 queue, so a serial terminal on COM1 becomes a byte source for `fd 0` with no
/// IDT/PIC plumbing. Mirrors `raw_byte_out`'s port discipline. Added for the devcockpit
/// DE serial console (a host terminal is already a decoded ANSI/UTF-8 byte stream, which
/// is exactly what `SYS_READ(fd=0)` consumers want).
pub fn serial_try_getc() -> Option<u8> {
    // SAFETY: reads LSR, then conditionally RBR, on COM1. Port IN has no memory effects;
    // reading RBR pops one byte from the 16550 RX FIFO.
    unsafe {
        let status: u8;
        core::arch::asm!(
            "in al, dx",
            in("dx") COM1_LSR,
            out("al") status,
            options(nostack, preserves_flags)
        );
        if status & LSR_RX_READY == 0 {
            return None;
        }
        let byte: u8;
        core::arch::asm!(
            "in al, dx",
            in("dx") COM1,
            out("al") byte,
            options(nostack, preserves_flags)
        );
        Some(byte)
    }
}

// === IRQ save/restore hooks (same-core lock reentrancy) =====================

/// Disable interrupts, returning the prior RFLAGS so `irq_restore` can undo it.
/// Installed via `morpheus_console::set_irq_guard`.
fn irq_save() -> u64 {
    let flags: u64;
    // SAFETY: PUSHF then CLI; reads RFLAGS off the stack. No memory clobber
    // beyond the transient push/pop.
    unsafe {
        core::arch::asm!(
            "pushf",
            "pop {0}",
            "cli",
            out(reg) flags,
            options(nostack),
        );
    }
    flags
}

/// Restore RFLAGS captured by `irq_save` (re-enables IF only if it was set).
fn irq_restore(state: u64) {
    // SAFETY: PUSH then POPF restores the caller's saved RFLAGS verbatim.
    unsafe {
        core::arch::asm!(
            "push {0}",
            "popf",
            in(reg) state,
            options(nostack),
        );
    }
}

// === Uptime clock hook (microseconds) =======================================

/// Monotonic uptime in microseconds from the TSC. Returns 0 until the TSC is
/// calibrated (`tsc_frequency() == 0`), so console lines stamped before the
/// "timing (tsc)" boot phase show `[    0.000000]` instead of garbage. Installed
/// via `morpheus_console::set_clock`.
fn uptime_us() -> u64 {
    let hz = crate::cpu::tsc::tsc_frequency();
    if hz == 0 {
        return 0;
    }
    let tsc = crate::cpu::tsc::read_tsc();
    // u128 to avoid overflow; tsc * 1e6 / hz.
    ((tsc as u128).saturating_mul(1_000_000) / (hz as u128)) as u64
}

// === Init ===================================================================

/// Program COM1 to a known 115200 8N1 polled config, independent of firmware,
/// and install the x86 seam hooks (byte-out + IRQ save/restore) into the
/// portable console. The console only ever writes THR / reads LSR via the
/// installed sink, so without this it would inherit whatever baud + line control
/// UEFI happened to leave — undefined across boards. Idempotent; BSP-only,
/// single-threaded early boot.
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

    // Wire the console to our UART + IRQ primitives. After this point the ring,
    // line lock, CRLF, and checkpoint all reach the port.
    morpheus_console::set_byte_sink(raw_byte_out);
    morpheus_console::set_irq_guard(irq_save, irq_restore);

    // Per-line uptime stamp. Safe to install now: the clock self-guards and
    // returns 0 until TSC calibration completes in the "timing (tsc)" phase.
    // (cpu_id is installed later, after per-cpu init, to avoid a gs: fault.)
    morpheus_console::set_clock(uptime_us);
}
