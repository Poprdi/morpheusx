//! PS/2 mouse driver — reads raw port 0x60/0x64 and parses packets.
//!
//! PS/2 mouse protocol (3-byte absolute packet):
//!   Byte 0: [YOVFL XOVFL YS XS 1 MB RB LB]  (sync bit=1, always)
//!   Byte 1: X delta (signed)
//!   Byte 2: Y delta (signed)
//!   Buttons: LB=bit0, RB=bit1, MB=bit2
//!
//! Controller initialization (done once at startup):
//!   - Enable port 2 (mouse)
//!   - Write config byte to allow port 2 data
//!   - Reset and enable mouse scanning

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;

const OBF: u8 = 0x01; // Output buffer full
const AUXB: u8 = 0x20; // Auxiliary device (mouse) data available

/// State machine for PS/2 mouse packet assembly.
static PACKET_STATE: AtomicU8 = AtomicU8::new(0); // 0=waiting for sync, 1-2=collecting bytes
static PACKET_BUF: [AtomicU8; 3] = [AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0)];

/// Initialization flag — run init once.
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize the PS/2 mouse controller (port 2).
/// Called once from the kernel startup to enable mouse I/O.
pub fn init() {
    if INITIALIZED.swap(true, Ordering::Relaxed) {
        return; // already initialized
    }

    unsafe {
        // Disable port 2 to avoid spurious data during init.
        write_cmd(0xA7); // disable port 2
        io_delay();
        flush_output();

        // Read the controller config byte.
        write_cmd(0x20); // read config byte
        let config = wait_response(50_000).unwrap_or(0x45);

        // Enable port 2 IRQ and clear port 1 IRQ
        let new_config = (config | 0x42) & !0x01; // set translation, enable aux IRQ, disable kbd IRQ

        write_cmd(0x60); // write config byte
        write_data(new_config);
        io_delay();

        // Enable port 2.
        write_cmd(0xA8); // enable port 2
        io_delay();

        // Reset the mouse.
        write_cmd(0xD4); // write to port 2
        write_data(0xFF); // mouse reset
        let _ = wait_response(50_000);
        flush_output();

        // Enable mouse data reporting.
        write_cmd(0xD4); // write to port 2
        write_data(0xF4); // enable scanning
        let _ = wait_response(50_000);
        flush_output();

        PACKET_STATE.store(0, Ordering::Relaxed);
    }
}

/// Poll the mouse port and accumulate motion/button data into the kernel accumulator.
/// Call this periodically (e.g., from the scheduler tick) to collect mouse input.
pub fn poll() {
    loop {
        let status = unsafe { read_status() };
        if (status & OBF) == 0 {
            break; // no data available
        }

        // Check if it's mouse data (AUXB=1) or keyboard data (AUXB=0)
        if (status & AUXB) == 0 {
            // Keyboard data, not for us
            let _ = unsafe { read_data() };
            continue;
        }

        // Mouse data: collect into packet buffer
        let byte = unsafe { read_data() };
        let state = PACKET_STATE.load(Ordering::Relaxed);

        match state {
            0 => {
                // Waiting for sync byte (bit 3 must be set)
                if (byte & 0x08) != 0 {
                    PACKET_BUF[0].store(byte, Ordering::Relaxed);
                    PACKET_STATE.store(1, Ordering::Relaxed);
                }
                // else: keep waiting for sync
            }
            1 => {
                // Collecting byte 1 (X delta)
                PACKET_BUF[1].store(byte, Ordering::Relaxed);
                PACKET_STATE.store(2, Ordering::Relaxed);
            }
            2 => {
                // Collected byte 2 (Y delta) — packet complete, process it
                PACKET_BUF[2].store(byte, Ordering::Relaxed);

                let flags = PACKET_BUF[0].load(Ordering::Relaxed);
                let dx_raw = PACKET_BUF[1].load(Ordering::Relaxed);
                let dy_raw = PACKET_BUF[2].load(Ordering::Relaxed);

                // Parse deltas (sign-extend from bytes)
                let dx = sign_extend_byte(dx_raw) as i16;
                let dy = sign_extend_byte(dy_raw) as i16;

                // Negate Y because PS/2 Y-down is positive but we want Y-up positive
                let dy = -dy;

                // Extract buttons
                let buttons = flags & 0x07; // bits [2:0] = MB, RB, LB

                // Accumulate into the global mouse state
                crate::mouse::accumulate(dx, dy, buttons);

                // Reset to waiting for next packet
                PACKET_STATE.store(0, Ordering::Relaxed);
            }
            _ => {
                PACKET_STATE.store(0, Ordering::Relaxed);
            }
        }
    }
}

/// Sign-extend an unsigned byte to a signed i32.
#[inline]
fn sign_extend_byte(b: u8) -> i32 {
    if (b & 0x80) != 0 {
        (b as i32) - 256
    } else {
        b as i32
    }
}

/// Read PS/2 status port (0x64).
#[inline]
unsafe fn read_status() -> u8 {
    let mut val: u8;
    core::arch::asm!("in al, dx", in("dx") PS2_STATUS, out("al") val);
    val
}

/// Read PS/2 data port (0x60).
#[inline]
unsafe fn read_data() -> u8 {
    let mut val: u8;
    core::arch::asm!("in al, dx", in("dx") PS2_DATA, out("al") val);
    val
}

/// Write command to PS/2 controller (port 0x64).
#[inline]
unsafe fn write_cmd(cmd: u8) {
    core::arch::asm!("out dx, al", in("dx") PS2_STATUS, in("al") cmd);
}

/// Write data to PS/2 controller (port 0x60).
#[inline]
unsafe fn write_data(data: u8) {
    core::arch::asm!("out dx, al", in("dx") PS2_DATA, in("al") data);
}

/// Flush the PS/2 output buffer.
unsafe fn flush_output() {
    for _ in 0..256 {
        if (read_status() & OBF) == 0 {
            break;
        }
        let _ = read_data();
    }
}

/// Wait for a response byte from the PS/2 controller.
unsafe fn wait_response(max_spins: u32) -> Option<u8> {
    for _ in 0..max_spins {
        if (read_status() & OBF) != 0 {
            return Some(read_data());
        }
        core::hint::spin_loop();
    }
    None
}

/// Tiny delay between commands (write to unused port 0x80).
#[inline]
unsafe fn io_delay() {
    core::arch::asm!(
        "out 0x80, al",
        out("al") _,
        options(nostack, preserves_flags, nomem),
    );
}
