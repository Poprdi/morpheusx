//! PS/2 mouse: 3-byte packet decoder with byte-0 sync, overflow clamp, 9-bit sign extend.

use super::input::{asm_ps2_flush, asm_ps2_poll_any, asm_ps2_write_cmd, asm_ps2_write_data};

#[derive(Clone, Copy, Default)]
pub struct MousePacket {
    pub dx: i16,
    pub dy: i16,
    /// bit0 L, bit1 R, bit2 M
    pub buttons: u8,
}

/// Consecutive byte-0 sync failures before resetting the packet assembler.
const RESYNC_THRESHOLD: u8 = 3;

pub struct Mouse {
    buf: [u8; 3],
    fill: usize,
    desync_count: u8,
}

impl Mouse {
    /// Skip aux-port init; for hardware with no working PS/2 mouse where the
    /// init path floods `mouse_cmd` timeouts.
    pub fn new_decoder_only() -> Self {
        Self {
            buf: [0; 3],
            fill: 0,
            desync_count: 0,
        }
    }

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

        // Enable aux port.
        asm_ps2_write_cmd(0xA8);
        io_delay();

        // RMW config: enable aux IRQ (bit1), clear aux clock-disable (bit5);
        // keep translation + kbd IRQ on.
        asm_ps2_write_cmd(0x20);
        let config = wait_kbd_data(50_000).unwrap_or(0x45);
        let new_config = (config | 0x43) & !0x30;
        asm_ps2_write_cmd(0x60);
        asm_ps2_write_data(new_config);
        io_delay();

        let ack_reset = mouse_cmd(0xFF);
        if ack_reset != 0xFA {
            morpheus_hal_x86_64::serial::log_warn("INPUT", 931, "mouse reset ACK missing");
        }
        // eat BAT + device ID
        drain(100_000);

        let ack_defaults = mouse_cmd(0xF6);
        if ack_defaults != 0xFA {
            morpheus_hal_x86_64::serial::log_warn("INPUT", 932, "mouse defaults ACK missing");
        }

        let ack_stream = mouse_cmd(0xF4);
        if ack_stream != 0xFA {
            morpheus_hal_x86_64::serial::log_warn("INPUT", 933, "mouse stream ACK missing");
        }

        asm_ps2_flush();
        self.fill = 0;
        self.desync_count = 0;
    }

    /// Returns a packet once 3 aux-port bytes are accumulated.
    /// Byte 0 must have bit 3 set (PS/2 always-1); resync on RESYNC_THRESHOLD misses.
    pub fn feed(&mut self, byte: u8) -> Option<MousePacket> {
        if self.fill == 0 {
            if byte & 0x08 == 0 {
                self.desync_count = self.desync_count.saturating_add(1);
                if self.desync_count >= RESYNC_THRESHOLD {
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

        // Bit-3-only sync is too weak with stale bytes at boot; cross-check sign bits.
        if !packet_sign_bits_match(status, raw_dx, raw_dy) {
            self.desync_count = self.desync_count.saturating_add(1);
            if self.desync_count >= RESYNC_THRESHOLD {
                self.desync_count = 0;
            }
            return None;
        }

        self.desync_count = 0;
        let buttons = status & 0x07;

        // 9-bit sign extend from status bits 4 (X) and 5 (Y).
        let mut dx = self.buf[1] as i16;
        if status & 0x10 != 0 {
            dx |= !0xFF;
        }

        let mut dy = self.buf[2] as i16;
        if status & 0x20 != 0 {
            dy |= !0xFF;
        }

        // Overflow (bits 6/7): delta byte is meaningless, clamp by sign.
        if status & 0x40 != 0 {
            dx = if status & 0x10 != 0 { -255 } else { 255 };
        }
        if status & 0x80 != 0 {
            dy = if status & 0x20 != 0 { -255 } else { 255 };
        }

        Some(MousePacket {
            dx,
            dy: -dy, // PS/2 Y is inverted vs screen
            buttons,
        })
    }
}

#[inline(always)]
fn packet_sign_bits_match(status: u8, dx: u8, dy: u8) -> bool {
    // Overflow makes data bytes unreliable; treat the check as passing.
    let x_overflow = (status & 0x40) != 0;
    let y_overflow = (status & 0x80) != 0;
    let x_ok = x_overflow || (((status & 0x10) != 0) == ((dx & 0x80) != 0));
    let y_ok = y_overflow || (((status & 0x20) != 0) == ((dy & 0x80) != 0));
    x_ok && y_ok
}

/// 0xD4 routes the next data byte to the aux port; returns ACK.
unsafe fn mouse_cmd(byte: u8) -> u8 {
    asm_ps2_write_cmd(0xD4);
    asm_ps2_write_data(byte);
    io_delay();
    wait_aux_data(50_000)
}

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

/// Spin-read one kbd-tagged byte (0x1xx) from 0x60.
unsafe fn wait_kbd_data(max_spins: u32) -> Option<u8> {
    for _ in 0..max_spins {
        let r = asm_ps2_poll_any();
        if (r & 0x300) == 0x100 {
            return Some((r & 0xFF) as u8);
        }
        core::hint::spin_loop();
    }
    None
}

/// Spin-read one aux-tagged byte (0x3xx) from 0x60.
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
