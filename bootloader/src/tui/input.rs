//! PS/2 Keyboard Driver — full ASCII, scan code set 1.
//!
//! Architecture mirrors every other driver in this repo:
//!   ASM primitives (bootloader/asm/keyboard/ps2.s)
//!     → Rust bindings (extern "win64")
//!       → Orchestration (init, poll, translate)
//!
//! Port 0x60 = data, port 0x64 = status/command.
//! Status bit 0 (OBF) = data ready. Bit 1 (IBF) = controller busy.
//!
//! We track shift, ctrl, alt, capslock as modifier state and produce
//! InputKey { scan_code, unicode_char } — same struct the TUI consumes.
//! scan_code uses EFI-compatible values so main_menu.rs needs zero changes.

// ═══════════════════════════════════════════════════════════════════════════
// ASM BINDINGS — from bootloader/asm/keyboard/ps2.s
// ═══════════════════════════════════════════════════════════════════════════

extern "win64" {
    /// Read PS/2 status register (port 0x64).
    fn asm_ps2_read_status() -> u8;
    /// Write command to controller (port 0x64), waits IBF=0.
    fn asm_ps2_write_cmd(cmd: u8);
    /// Write data byte to controller (port 0x60), waits IBF=0.
    fn asm_ps2_write_data(data: u8);
    /// Non-blocking poll: 0 = empty, 0x1xx = keyboard byte xx.
    fn asm_ps2_poll() -> u32;
    /// Drain output buffer (up to 256 reads).
    fn asm_ps2_flush();
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC KEY TYPE
// ═══════════════════════════════════════════════════════════════════════════

/// Key event. scan_code != 0 for special keys, unicode_char != 0 for ASCII.
/// Both can be set simultaneously (e.g. Enter = scan_code 0, unicode_char 0x0D).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct InputKey {
    pub scan_code: u16,
    pub unicode_char: u16,
}

// ── EFI-compatible scan codes (main_menu.rs expects these exact values) ──

pub const SCAN_NULL: u16 = 0x00;
pub const SCAN_UP: u16 = 0x01;
pub const SCAN_DOWN: u16 = 0x02;
pub const SCAN_RIGHT: u16 = 0x03;
pub const SCAN_LEFT: u16 = 0x04;
pub const SCAN_HOME: u16 = 0x05;
pub const SCAN_END: u16 = 0x06;
pub const SCAN_INSERT: u16 = 0x07;
pub const SCAN_DELETE: u16 = 0x08;
pub const SCAN_PGUP: u16 = 0x09;
pub const SCAN_PGDN: u16 = 0x0A;
pub const SCAN_F1: u16 = 0x0B;
pub const SCAN_F2: u16 = 0x0C;
pub const SCAN_F3: u16 = 0x0D;
pub const SCAN_F4: u16 = 0x0E;
pub const SCAN_F5: u16 = 0x0F;
pub const SCAN_F6: u16 = 0x10;
pub const SCAN_F7: u16 = 0x11;
pub const SCAN_F8: u16 = 0x12;
pub const SCAN_F9: u16 = 0x13;
pub const SCAN_F10: u16 = 0x14;
pub const SCAN_F11: u16 = 0x15;
pub const SCAN_F12: u16 = 0x16;
pub const SCAN_ESC: u16 = 0x17;

// ── ASCII codes ──

pub const KEY_ENTER: u16 = 0x0D;
pub const KEY_SPACE: u16 = 0x20;
pub const KEY_TAB: u16 = 0x09;
pub const KEY_BACKSPACE: u16 = 0x08;

// ═══════════════════════════════════════════════════════════════════════════
// SCAN CODE SET 1 → ASCII TRANSLATION TABLES
// ═══════════════════════════════════════════════════════════════════════════

/// Unshifted ASCII for scan code set 1 make codes 0x00..0x58.
/// 0 = no printable character (modifier or special key).
#[rustfmt::skip]
static UNSHIFTED: [u8; 89] = [
//  0x00  0x01  0x02  0x03  0x04  0x05  0x06  0x07
    0,    0,    b'1', b'2', b'3', b'4', b'5', b'6',  // 0x00 Esc=special
//  0x08  0x09  0x0A  0x0B  0x0C  0x0D  0x0E  0x0F
    b'7', b'8', b'9', b'0', b'-', b'=', 0x08, 0x09,  // BS=0x08 Tab=0x09
//  0x10  0x11  0x12  0x13  0x14  0x15  0x16  0x17
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i',
//  0x18  0x19  0x1A  0x1B  0x1C  0x1D  0x1E  0x1F
    b'o', b'p', b'[', b']', 0x0D, 0,    b'a', b's',  // Enter=0x0D LCtrl=0
//  0x20  0x21  0x22  0x23  0x24  0x25  0x26  0x27
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';',
//  0x28  0x29  0x2A  0x2B  0x2C  0x2D  0x2E  0x2F
    b'\'',b'`', 0,    b'\\',b'z', b'x', b'c', b'v',  // LShift=0
//  0x30  0x31  0x32  0x33  0x34  0x35  0x36  0x37
    b'b', b'n', b'm', b',', b'.', b'/', 0,    b'*',  // RShift=0 KP*
//  0x38  0x39  0x3A  0x3B  0x3C  0x3D  0x3E  0x3F
    0,    b' ', 0,    0,    0,    0,    0,    0,      // LAlt CapsLk F1-F5
//  0x40  0x41  0x42  0x43  0x44  0x45  0x46  0x47
    0,    0,    0,    0,    0,    0,    0,    b'7',   // F6-F10 Num Scr KP7
//  0x48  0x49  0x4A  0x4B  0x4C  0x4D  0x4E  0x4F
    b'8', b'9', b'-', b'4', b'5', b'6', b'+', b'1',  // KP row
//  0x50  0x51  0x52  0x53  0x54  0x55  0x56  0x57
    b'2', b'3', b'0', b'.', 0,    0,    0,    0,     // KP row .. F11
//  0x58
    0,                                                 // F12
];

/// Shifted ASCII for scan code set 1 make codes 0x00..0x58.
#[rustfmt::skip]
static SHIFTED: [u8; 89] = [
//  0x00  0x01  0x02  0x03  0x04  0x05  0x06  0x07
    0,    0,    b'!', b'@', b'#', b'$', b'%', b'^',
//  0x08  0x09  0x0A  0x0B  0x0C  0x0D  0x0E  0x0F
    b'&', b'*', b'(', b')', b'_', b'+', 0x08, 0x09,
//  0x10  0x11  0x12  0x13  0x14  0x15  0x16  0x17
    b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I',
//  0x18  0x19  0x1A  0x1B  0x1C  0x1D  0x1E  0x1F
    b'O', b'P', b'{', b'}', 0x0D, 0,    b'A', b'S',
//  0x20  0x21  0x22  0x23  0x24  0x25  0x26  0x27
    b'D', b'F', b'G', b'H', b'J', b'K', b'L', b':',
//  0x28  0x29  0x2A  0x2B  0x2C  0x2D  0x2E  0x2F
    b'"', b'~', 0,    b'|', b'Z', b'X', b'C', b'V',
//  0x30  0x31  0x32  0x33  0x34  0x35  0x36  0x37
    b'B', b'N', b'M', b'<', b'>', b'?', 0,    b'*',
//  0x38  0x39  0x3A  0x3B  0x3C  0x3D  0x3E  0x3F
    0,    b' ', 0,    0,    0,    0,    0,    0,
//  0x40  0x41  0x42  0x43  0x44  0x45  0x46  0x47
    0,    0,    0,    0,    0,    0,    0,    b'7',
//  0x48  0x49  0x4A  0x4B  0x4C  0x4D  0x4E  0x4F
    b'8', b'9', b'-', b'4', b'5', b'6', b'+', b'1',
//  0x50  0x51  0x52  0x53  0x54  0x55  0x56  0x57
    b'2', b'3', b'0', b'.', 0,    0,    0,    0,
//  0x58
    0,
];

// ═══════════════════════════════════════════════════════════════════════════
// PS/2 SCAN CODE CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

// Make codes (key press)
const SC_ESC: u8         = 0x01;
const SC_LCTRL: u8       = 0x1D;
const SC_LSHIFT: u8      = 0x2A;
const SC_RSHIFT: u8      = 0x36;
const SC_LALT: u8        = 0x38;
const SC_CAPSLOCK: u8    = 0x3A;
const SC_F1: u8          = 0x3B;
const SC_F10: u8         = 0x44;
const SC_F11: u8         = 0x57;
const SC_F12: u8         = 0x58;

// Extended (0xE0 prefix) make codes
const SC_EXT_UP: u8      = 0x48;
const SC_EXT_DOWN: u8    = 0x50;
const SC_EXT_LEFT: u8    = 0x4B;
const SC_EXT_RIGHT: u8   = 0x4D;
const SC_EXT_HOME: u8    = 0x47;
const SC_EXT_END: u8     = 0x4F;
const SC_EXT_PGUP: u8    = 0x49;
const SC_EXT_PGDN: u8    = 0x51;
const SC_EXT_INSERT: u8  = 0x52;
const SC_EXT_DELETE: u8  = 0x53;

// Break code flag
const BREAK_FLAG: u8     = 0x80;

// Extended prefix byte
const EXTENDED_PREFIX: u8 = 0xE0;

// ═══════════════════════════════════════════════════════════════════════════
// KEYBOARD STATE
// ═══════════════════════════════════════════════════════════════════════════

pub struct Keyboard {
    /// Shift held (left or right)
    shift: bool,
    /// Ctrl held
    ctrl: bool,
    /// Alt held
    alt: bool,
    /// Caps lock toggled
    caps_lock: bool,
    /// Next byte is an extended (0xE0) scancode
    extended: bool,
    /// Controller initialized
    initialized: bool,
}

impl Keyboard {
    /// Initialize the PS/2 keyboard controller.
    ///
    /// Sequence:
    ///   1. Flush stale data (UEFI may have left bytes in buffer)
    ///   2. Disable scanning so init commands don't get mixed with keycodes
    ///   3. Controller self-test
    ///   4. Enable first PS/2 port
    ///   5. Reset keyboard device
    ///   6. Set scan code set 1 (explicit — don't trust BIOS/UEFI default)
    ///   7. Enable scanning
    ///   8. Flush again (init may have generated ACK/response bytes)
    pub fn new() -> Self {
        let mut kb = Self {
            shift: false,
            ctrl: false,
            alt: false,
            caps_lock: false,
            extended: false,
            initialized: false,
        };

        unsafe { kb.init_controller() };
        kb
    }

    unsafe fn init_controller(&mut self) {
        use morpheus_hwinit::serial::puts;

        // 1. Flush any stale bytes
        asm_ps2_flush();

        // 2. Disable first PS/2 port (no scancodes during init)
        asm_ps2_write_cmd(0xAD);
        Self::io_delay();

        // 3. Flush again after disable
        asm_ps2_flush();

        // 4. Controller self-test (0xAA → expects 0x55 response)
        asm_ps2_write_cmd(0xAA);
        let response = Self::wait_response(50_000);
        if response != Some(0x55) {
            puts("[KBD] controller self-test failed, continuing anyway\n");
        }

        // 5. Enable first PS/2 port
        asm_ps2_write_cmd(0xAE);
        Self::io_delay();

        // 6. Reset keyboard device (0xFF → expects 0xFA ACK, then 0xAA)
        asm_ps2_write_data(0xFF);
        let ack = Self::wait_response(100_000);
        if ack == Some(0xFA) {
            // Wait for self-test pass (0xAA) or fail (0xFC)
            let _st = Self::wait_response(500_000);
        }

        // 7. Set scan code set 1: send 0xF0 (select set), then 0x01
        asm_ps2_write_data(0xF0);
        let _ = Self::wait_response(50_000); // ACK
        asm_ps2_write_data(0x01);
        let _ = Self::wait_response(50_000); // ACK

        // 8. Enable scanning (0xF4)
        asm_ps2_write_data(0xF4);
        let _ = Self::wait_response(50_000); // ACK

        // 9. Final flush — discard init noise
        asm_ps2_flush();

        self.initialized = true;
        puts("[KBD] PS/2 keyboard ready (scan set 1, full ASCII)\n");
    }

    /// Spin-wait for a response byte from port 0x60, with bounded timeout.
    unsafe fn wait_response(max_spins: u32) -> Option<u8> {
        for _ in 0..max_spins {
            let r = asm_ps2_poll();
            if r & 0x100 != 0 {
                return Some((r & 0xFF) as u8);
            }
            core::hint::spin_loop();
        }
        None
    }

    /// Tiny delay between commands (write to unused port 0x80, like PIC does).
    #[inline(always)]
    unsafe fn io_delay() {
        // Same pattern as hwinit/src/cpu/pic.rs io_wait()
        core::arch::asm!(
            "out 0x80, al",
            out("al") _,
            options(nostack, preserves_flags, nomem),
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // PUBLIC API (unchanged signatures — TUI code needs zero changes)
    // ═══════════════════════════════════════════════════════════════════════

    /// Non-blocking key read. Returns `Some(InputKey)` if a complete
    /// key-press event was decoded, `None` if the buffer was empty or
    /// only a modifier/break code was processed.
    pub fn read_key(&mut self) -> Option<InputKey> {
        let raw = unsafe { asm_ps2_poll() };
        if raw & 0x100 == 0 {
            return None; // OBF empty
        }
        let byte = (raw & 0xFF) as u8;
        self.decode(byte)
    }

    /// Blocking wait — spins until a printable/actionable key arrives.
    pub fn wait_for_key(&mut self) -> InputKey {
        loop {
            if let Some(key) = self.read_key() {
                return key;
            }
            // ~10k iterations ≈ a few µs on modern CPUs
            for _ in 0..10_000 {
                core::hint::spin_loop();
            }
        }
    }

    /// Poll with ~16ms delay for animation loops (~60Hz frame pacing).
    pub fn poll_key_with_delay(&mut self) -> Option<InputKey> {
        let key = self.read_key();
        // Spin delay: ~16ms at ~1GHz-ish effective throughput.
        // Not precise, but good enough for 60Hz frame pacing.
        for _ in 0..400_000 {
            core::hint::spin_loop();
        }
        key
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SCAN CODE DECODER
    // ═══════════════════════════════════════════════════════════════════════

    /// Decode a single raw byte from port 0x60.
    ///
    /// Returns `Some(InputKey)` only on complete key-press events.
    /// Modifier transitions (shift down/up, ctrl, alt, capslock toggle)
    /// and break codes return `None` — they update internal state only.
    fn decode(&mut self, byte: u8) -> Option<InputKey> {
        // Extended prefix — remember it, consume the byte
        if byte == EXTENDED_PREFIX {
            self.extended = true;
            return None;
        }

        let is_break = byte & BREAK_FLAG != 0;
        let make = byte & !BREAK_FLAG; // strip break bit to get make code

        if self.extended {
            self.extended = false;
            return self.decode_extended(make, is_break);
        }

        // ── Break codes (key release) — update modifiers, emit nothing ──
        if is_break {
            match make {
                SC_LSHIFT | SC_RSHIFT => self.shift = false,
                SC_LCTRL              => self.ctrl = false,
                SC_LALT               => self.alt = false,
                _                     => {} // ignore other releases
            }
            return None;
        }

        // ── Make codes (key press) ──

        // Modifiers
        match make {
            SC_LSHIFT | SC_RSHIFT => { self.shift = true; return None; }
            SC_LCTRL              => { self.ctrl = true; return None; }
            SC_LALT               => { self.alt = true; return None; }
            SC_CAPSLOCK           => { self.caps_lock = !self.caps_lock; return None; }
            _ => {}
        }

        // Escape
        if make == SC_ESC {
            return Some(InputKey { scan_code: SCAN_ESC, unicode_char: 0 });
        }

        // Function keys F1-F10
        if make >= SC_F1 && make <= SC_F10 {
            let fkey = SCAN_F1 + (make - SC_F1) as u16;
            return Some(InputKey { scan_code: fkey, unicode_char: 0 });
        }

        // F11, F12
        if make == SC_F11 {
            return Some(InputKey { scan_code: SCAN_F11, unicode_char: 0 });
        }
        if make == SC_F12 {
            return Some(InputKey { scan_code: SCAN_F12, unicode_char: 0 });
        }

        // ASCII from translation table
        if (make as usize) < UNSHIFTED.len() {
            let ch = self.translate_ascii(make);
            if ch != 0 {
                // Ctrl+letter produces control characters (0x01-0x1A)
                let unicode = if self.ctrl && ch >= b'a' && ch <= b'z' {
                    (ch - b'a' + 1) as u16
                } else if self.ctrl && ch >= b'A' && ch <= b'Z' {
                    (ch - b'A' + 1) as u16
                } else {
                    ch as u16
                };
                return Some(InputKey { scan_code: SCAN_NULL, unicode_char: unicode });
            }
        }

        None // unrecognized or non-printable
    }

    /// Decode an extended (0xE0-prefixed) scancode.
    fn decode_extended(&mut self, make: u8, is_break: bool) -> Option<InputKey> {
        // Extended modifier releases (right ctrl, right alt)
        if is_break {
            match make {
                SC_LCTRL => self.ctrl = false,  // Right Ctrl shares make code
                SC_LALT  => self.alt = false,   // Right Alt shares make code
                _ => {}
            }
            return None;
        }

        // Extended modifier presses
        match make {
            SC_LCTRL => { self.ctrl = true; return None; }
            SC_LALT  => { self.alt = true; return None; }
            _ => {}
        }

        // Navigation keys → EFI scan codes
        let scan = match make {
            SC_EXT_UP     => SCAN_UP,
            SC_EXT_DOWN   => SCAN_DOWN,
            SC_EXT_LEFT   => SCAN_LEFT,
            SC_EXT_RIGHT  => SCAN_RIGHT,
            SC_EXT_HOME   => SCAN_HOME,
            SC_EXT_END    => SCAN_END,
            SC_EXT_PGUP   => SCAN_PGUP,
            SC_EXT_PGDN   => SCAN_PGDN,
            SC_EXT_INSERT => SCAN_INSERT,
            SC_EXT_DELETE => SCAN_DELETE,
            _ => return None,
        };

        Some(InputKey { scan_code: scan, unicode_char: 0 })
    }

    /// Look up ASCII from scan code, applying shift and caps lock.
    ///
    /// Caps Lock only affects a-z / A-Z. Shift toggles everything.
    /// When both caps lock and shift are active, they cancel for letters
    /// (standard US keyboard behavior).
    fn translate_ascii(&self, make: u8) -> u8 {
        let idx = make as usize;
        let base = UNSHIFTED[idx];

        // Non-letter keys: shift selects table directly
        let is_letter = base.is_ascii_lowercase();

        if is_letter {
            // Letters: shift XOR capslock determines case
            let upper = self.shift ^ self.caps_lock;
            if upper { SHIFTED[idx] } else { base }
        } else {
            // Everything else: shift only
            if self.shift { SHIFTED[idx] } else { base }
        }
    }
}
