//! PS/2 mouse driver — 3-byte packet decoder, relative motion.

use super::input::{asm_ps2_flush, asm_ps2_poll_any, asm_ps2_write_cmd, asm_ps2_write_data};

#[derive(Clone, Copy, Default)]
pub struct MousePacket {
    pub dx: i16,
    pub dy: i16,
    /// Bit 0 = left, bit 1 = right, bit 2 = middle
    pub buttons: u8,
}

pub struct Mouse {
    buf: [u8; 3],
    fill: usize,
}

impl Mouse {
    pub fn new() -> Self {
        let mut m = Self { buf: [0; 3], fill: 0 };
        unsafe { m.init() };
        m
    }

    unsafe fn init(&mut self) {
        use morpheus_hwinit::serial::puts;

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
        mouse_cmd(0xFF);
        // eat BAT result + device ID
        drain(100_000);

        // Set defaults
        mouse_cmd(0xF6);

        // Enable streaming
        mouse_cmd(0xF4);

        asm_ps2_flush();
        self.fill = 0;
        puts("[MOUSE] PS/2 mouse ready\n");
    }

    /// Feed one raw byte from PS/2 aux port. Returns packet when 3 bytes complete.
    pub fn feed(&mut self, byte: u8) -> Option<MousePacket> {
        // Byte 0 must have bit 3 set (always-1 in PS/2 status byte)
        if self.fill == 0 && byte & 0x08 == 0 {
            return None;
        }

        self.buf[self.fill] = byte;
        self.fill += 1;

        if self.fill < 3 {
            return None;
        }
        self.fill = 0;

        let status = self.buf[0];

        // Discard on overflow
        if status & 0xC0 != 0 {
            return None;
        }

        let buttons = status & 0x07;

        // 9-bit sign extension: bit 4 of status = X sign, bit 5 = Y sign
        let mut dx = self.buf[1] as i16;
        if status & 0x10 != 0 { dx |= !0xFF; }

        let mut dy = self.buf[2] as i16;
        if status & 0x20 != 0 { dy |= !0xFF; }

        Some(MousePacket {
            dx,
            dy: -dy, // PS/2 Y is inverted vs screen coords
            buttons,
        })
    }
}

/// Send a command byte to the mouse (via 0xD4 → aux port), wait for ACK.
unsafe fn mouse_cmd(byte: u8) {
    asm_ps2_write_cmd(0xD4);
    asm_ps2_write_data(byte);
    io_delay();
    let _ = wait_data(50_000);
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

/// Drain a few responses (eat BAT, device IDs, etc.)
unsafe fn drain(max_spins: u32) {
    for _ in 0..max_spins {
        let r = asm_ps2_poll_any();
        if r == 0 { break; }
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
