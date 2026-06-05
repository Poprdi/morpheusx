//! Portable console/log subsystem: a 64 KiB boot-log ring + a single per-line
//! lock that funnels every producer (`puts`, `log_*`, `boot_step_*`) through one
//! atomic chokepoint. Platform specifics (UART byte-out, IRQ save/restore) are
//! installed as fn-pointer hooks; until set, output falls back to ring-only.
//! SMP-safe via CAS on the log index + an internal AtomicBool spin on the port.

#![no_std]

mod boot;
mod checkpoint;
mod fmt;
mod levels;
mod lock;
mod ring;
mod sink;
mod writer;

pub use boot::{boot_banner, boot_step_fail, boot_step_ok, boot_step_warn};
pub use checkpoint::{checkpoint, serial_putc, serial_puts, set_checkpoints_enabled};
pub use fmt::{
    put_hex32, put_hex64, put_hex8, puts_dec_u32, puts_dec_u8, puts_hex_u32, puts_hex_u64,
    puts_hex_u8,
};
pub use levels::{log_error, log_info, log_ok, log_warn, set_log_style};
pub use ring::boot_log;
pub use sink::{
    clear_live_console_hook, set_byte_sink, set_clock, set_cpu_id, set_fb_sink, set_irq_guard,
    set_live_console_hook,
};
pub use writer::{fb_putc, fb_puts, line, newline, putc, puts, LineWriter};
