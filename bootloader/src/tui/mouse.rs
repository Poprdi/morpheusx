//! PS/2 mouse driver — 3-byte packet decoder, relative motion.
//!
//! This is the single authoritative mouse driver for MorpheusX.
//! The bootloader event loop (`desktop.rs`) polls `asm_ps2_poll_any()`
//! and feeds aux-port bytes here via `Mouse::feed()`.
//!
//! Robustness features:
//!   - Byte-0 sync validation (bit 3 must be set)
//!   - Resync after 3 consecutive failed sync bytes (drops partial state)
//!   - Overflow detection (status bits 6-7) — clamp to ±255 instead of
//!     silently discarding the packet
//!   - Proper 9-bit sign extension using status-byte sign bits (4, 5)

use super::input::{asm_ps2_flush, asm_ps2_poll_any, asm_ps2_write_cmd, asm_ps2_write_data};

#[derive(Clone, Copy, Default)]
pub struct MousePacket {
    pub dx: i16,
    pub dy: i16,
    /// Bit 0 = left, bit 1 = right, bit 2 = middle
    pub buttons: u8,
}

/// Maximum number of out-of-sync bytes before we force a state reset.
/// After this many failed byte-0 syncs in a row, any partial packet is
/// discarded and we restart from byte 0.
const RESYNC_THRESHOLD: u8 = 3;

pub struct Mouse {
    buf: [u8; 3],
    fill: usize,
    /// Consecutive byte-0 sync failures (bit 3 not set).
    desync_count: u8,
}

impl Mouse {
    pub fn new() -> Self {
        let mut m = Self {
            buf: [0; 3],
            fill: 0,
            desync_count: 0,
        };
        unsafe { m.init() };
        m
    }

    unsafe fn init(&mut self) {
        asm_ps2_flush();

        // Enable aux port (port 2)
        asm_ps2_write_cmd(0xA8);
        io_delay();

        // RMW config: enable aux IRQ (bit 1), clear aux clock-disable (bit 5)
        asm_ps2_write_cmd(0x20);
        let config = wait_data(50_000);
        let new_config = (config | 0x02) & !0x20;
        asm_ps2_write_cmd(0x60);
        asm_ps2_write_data(new_config);
        io_delay();

        // Reset mouse (0xFF via aux)
        let ack_reset = mouse_cmd(0xFF);
        if ack_reset != 0xFA {
            morpheus_hwinit::serial::log_warn("INPUT", 931, "mouse reset ACK missing");
        }
        // eat BAT result + device ID
        drain(100_000);

        // Set defaults
        let ack_defaults = mouse_cmd(0xF6);
        if ack_defaults != 0xFA {
            morpheus_hwinit::serial::log_warn("INPUT", 932, "mouse defaults ACK missing");
        }

        // Enable streaming
        let ack_stream = mouse_cmd(0xF4);
        if ack_stream != 0xFA {
            morpheus_hwinit::serial::log_warn("INPUT", 933, "mouse stream ACK missing");
        }

        asm_ps2_flush();
        self.fill = 0;
        self.desync_count = 0;
        morpheus_hwinit::serial::log_ok("INPUT", 934, "PS/2 mouse ready");
    }

    /// Feed one raw byte from PS/2 aux port. Returns packet when 3 bytes complete.
    ///
    /// Sync strategy:
    ///   - Byte 0 MUST have bit 3 set (PS/2 always-1 status bit).
    ///   - If bit 3 is not set and we're waiting for byte 0, increment
    ///     `desync_count`.  After `RESYNC_THRESHOLD` failures, force-reset
    ///     the packet assembler to re-establish sync.
    ///   - Overflow (status bits 6-7) is handled by clamping deltas to
    ///     max magnitude (±255) instead of discarding the packet.
    pub fn feed(&mut self, byte: u8) -> Option<MousePacket> {
        // Byte 0 must have bit 3 set (always-1 in PS/2 status byte)
        if self.fill == 0 {
            if byte & 0x08 == 0 {
                self.desync_count = self.desync_count.saturating_add(1);
                if self.desync_count >= RESYNC_THRESHOLD {
                    // Force resync — reset state machine
                    self.desync_count = 0;
                }
                return None;
            }
            self.desync_count = 0;
        }

        self.buf[self.fill] = byte;
        self.fill += 1;

        if self.fill < 3 {
            return None;
        }
        self.fill = 0;

        let status = self.buf[0];
        let raw_dx = self.buf[1];
        let raw_dy = self.buf[2];

        // bit3-only sync is too weak when stale bytes exist at boot.
        // reject packets whose sign bits cannot describe the data bytes.
        if !packet_sign_bits_match(status, raw_dx, raw_dy) {
            self.desync_count = self.desync_count.saturating_add(1);
            if self.desync_count >= RESYNC_THRESHOLD {
                self.desync_count = 0;
            }
            return None;
        }

        self.desync_count = 0;
        let buttons = status & 0x07;

        // 9-bit sign extension using status byte sign bits.
        // Bit 4 = X sign, bit 5 = Y sign.
        let mut dx = self.buf[1] as i16;
        if status & 0x10 != 0 {
            dx |= !0xFF; // sign-extend: 0xFF00 | dx
        }

        let mut dy = self.buf[2] as i16;
        if status & 0x20 != 0 {
            dy |= !0xFF;
        }

        // Handle overflow: status bits 6 (X overflow) and 7 (Y overflow).
        // On overflow the delta byte is meaningless. Clamp to max magnitude
        // in the direction indicated by the sign bit.
        if status & 0x40 != 0 {
            // X overflow
            dx = if status & 0x10 != 0 { -255 } else { 255 };
        }
        if status & 0x80 != 0 {
            // Y overflow
            dy = if status & 0x20 != 0 { -255 } else { 255 };
        }

        Some(MousePacket {
            dx,
            dy: -dy, // PS/2 Y is inverted vs screen coords
            buttons,
        })
    }
}

#[inline(always)]
fn packet_sign_bits_match(status: u8, dx: u8, dy: u8) -> bool {
    // If overflow is set, data bytes are not reliable for sign checks.
    let x_overflow = (status & 0x40) != 0;
    let y_overflow = (status & 0x80) != 0;
    let x_ok = x_overflow || (((status & 0x10) != 0) == ((dx & 0x80) != 0));
    let y_ok = y_overflow || (((status & 0x20) != 0) == ((dy & 0x80) != 0));
    x_ok && y_ok
}

/// Send a command byte to the mouse (via 0xD4 → aux port), wait for ACK.
unsafe fn mouse_cmd(byte: u8) -> u8 {
    asm_ps2_write_cmd(0xD4);
    asm_ps2_write_data(byte);
    io_delay();
    wait_aux_data(50_000)
}

/// Spin-read one byte from port 0x60.
unsafe fn wait_data(max_spins: u32) -> u8 {
    for _ in 0..max_spins {
        let r = asm_ps2_poll_any();
        if r & 0x100 != 0 {
            return (r & 0xFF) as u8;
        }
        core::hint::spin_loop();
    }
    0
}

/// Spin-read one aux-port byte (mouse channel, tagged as 0x3xx).
unsafe fn wait_aux_data(max_spins: u32) -> u8 {
    for _ in 0..max_spins {
        let r = asm_ps2_poll_any();
        if (r & 0x300) == 0x300 {
            return (r & 0xFF) as u8;
        }
        core::hint::spin_loop();
    }
    0
}

/// Drain a few responses (eat BAT, device IDs, etc.)
unsafe fn drain(max_spins: u32) {
    for _ in 0..max_spins {
        let r = asm_ps2_poll_any();
        if r == 0 {
            break;
        }
        core::hint::spin_loop();
    }
}

#[inline(always)]
unsafe fn io_delay() {
    core::arch::asm!(
        "out 0x80, al",
        out("al") _,
        options(nostack, preserves_flags, nomem),
    );
}
