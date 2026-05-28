//! PS/2 keyboard — scan set 1, US/DE keymaps, runtime-switchable.
//! Ports 0x60 (data), 0x64 (status/cmd). Status: bit0 OBF, bit1 IBF.
//! Emits `InputKey { scan_code, unicode_char }` with EFI-compatible scan codes.

extern "win64" {
    fn asm_ps2_read_status() -> u8;
    pub fn asm_ps2_write_cmd(cmd: u8);
    pub fn asm_ps2_write_data(data: u8);
    fn asm_ps2_poll() -> u32;
    /// 0=empty, 0x1xx=kbd byte, 0x3xx=mouse byte.
    pub fn asm_ps2_poll_any() -> u32;
    pub fn asm_ps2_flush();
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct InputKey {
    pub scan_code: u16,
    pub unicode_char: u16,
}

// EFI scan codes — main_menu.rs depends on these exact values.

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

pub const KEY_ENTER: u16 = 0x0D;
pub const KEY_SPACE: u16 = 0x20;
pub const KEY_TAB: u16 = 0x09;
pub const KEY_BACKSPACE: u16 = 0x08;

#[derive(Clone, Copy, PartialEq)]
pub enum KeyLayout {
    Us,
    De,
}

// Scan set 1 → ASCII, make codes 0x00..0x58. Zero entries = non-printable.
#[rustfmt::skip]
static US_UNSHIFTED: [u8; 89] = [
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

#[rustfmt::skip]
static US_SHIFTED: [u8; 89] = [
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

// DE QWERTZ divergences vs US: y↔z, ß(0xDF) ü(0xFC) ö(0xF6) ä(0xE4) on the
// usual keys, dead accent/caret emitted literal, plus the 0x56 < > | extra.
// Umlauts use Latin-1 codepoints (fit in u8).

#[rustfmt::skip]
static DE_UNSHIFTED: [u8; 89] = [
//  0x00  0x01  0x02  0x03  0x04  0x05  0x06  0x07
    0,    0,    b'1', b'2', b'3', b'4', b'5', b'6',
//  0x08  0x09  0x0A  0x0B  0x0C  0x0D  0x0E  0x0F
    b'7', b'8', b'9', b'0', 0xDF, b'\'',0x08, 0x09,  // ß  dead-accent→'
//  0x10  0x11  0x12  0x13  0x14  0x15  0x16  0x17
    b'q', b'w', b'e', b'r', b't', b'z', b'u', b'i',  // y↔z
//  0x18  0x19  0x1A  0x1B  0x1C  0x1D  0x1E  0x1F
    b'o', b'p', 0xFC, b'+', 0x0D, 0,    b'a', b's',  // ü
//  0x20  0x21  0x22  0x23  0x24  0x25  0x26  0x27
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', 0xF6,  // ö
//  0x28  0x29  0x2A  0x2B  0x2C  0x2D  0x2E  0x2F
    0xE4, b'^', 0,    b'#', b'y', b'x', b'c', b'v',  // ä  ^ dead  y↔z
//  0x30  0x31  0x32  0x33  0x34  0x35  0x36  0x37
    b'b', b'n', b'm', b',', b'.', b'-', 0,    b'*',
//  0x38  0x39  0x3A  0x3B  0x3C  0x3D  0x3E  0x3F
    0,    b' ', 0,    0,    0,    0,    0,    0,
//  0x40  0x41  0x42  0x43  0x44  0x45  0x46  0x47
    0,    0,    0,    0,    0,    0,    0,    b'7',
//  0x48  0x49  0x4A  0x4B  0x4C  0x4D  0x4E  0x4F
    b'8', b'9', b'-', b'4', b'5', b'6', b'+', b'1',
//  0x50  0x51  0x52  0x53  0x54  0x55  0x56  0x57
    b'2', b'3', b'0', b'.', 0,    0,    b'<', 0,     // 0x56 extra key
//  0x58
    0,
];

#[rustfmt::skip]
static DE_SHIFTED: [u8; 89] = [
//  0x00  0x01  0x02  0x03  0x04  0x05  0x06  0x07
    0,    0,    b'!', b'"', 0xA7, b'$', b'%', b'&',  // §=0xA7
//  0x08  0x09  0x0A  0x0B  0x0C  0x0D  0x0E  0x0F
    b'/', b'(', b')', b'=', b'?', b'`', 0x08, 0x09,
//  0x10  0x11  0x12  0x13  0x14  0x15  0x16  0x17
    b'Q', b'W', b'E', b'R', b'T', b'Z', b'U', b'I',
//  0x18  0x19  0x1A  0x1B  0x1C  0x1D  0x1E  0x1F
    b'O', b'P', 0xDC, b'*', 0x0D, 0,    b'A', b'S',  // Ü=0xDC
//  0x20  0x21  0x22  0x23  0x24  0x25  0x26  0x27
    b'D', b'F', b'G', b'H', b'J', b'K', b'L', 0xD6,  // Ö=0xD6
//  0x28  0x29  0x2A  0x2B  0x2C  0x2D  0x2E  0x2F
    0xC4, b'`', 0,    b'\'',b'Y', b'X', b'C', b'V',  // Ä=0xC4
//  0x30  0x31  0x32  0x33  0x34  0x35  0x36  0x37
    b'B', b'N', b'M', b';', b':', b'_', 0,    b'*',
//  0x38  0x39  0x3A  0x3B  0x3C  0x3D  0x3E  0x3F
    0,    b' ', 0,    0,    0,    0,    0,    0,
//  0x40  0x41  0x42  0x43  0x44  0x45  0x46  0x47
    0,    0,    0,    0,    0,    0,    0,    b'7',
//  0x48  0x49  0x4A  0x4B  0x4C  0x4D  0x4E  0x4F
    b'8', b'9', b'-', b'4', b'5', b'6', b'+', b'1',
//  0x50  0x51  0x52  0x53  0x54  0x55  0x56  0x57
    b'2', b'3', b'0', b'.', 0,    0,    b'>', 0,     // 0x56 shifted >
//  0x58
    0,
];

/// DE AltGr layer; 0 = unmapped.
#[rustfmt::skip]
static DE_ALTGR: [u8; 89] = [
//  0x00  0x01  0x02  0x03  0x04  0x05  0x06  0x07
    0,    0,    0,    b'@', 0,    0,    0,    0,      // 2→@ (on some physical DE kbs)
//  0x08  0x09  0x0A  0x0B  0x0C  0x0D  0x0E  0x0F
    b'{', b'[', b']', b'}', b'\\',0,    0,    0,     // 7{ 8[ 9] 0} ß→\
//  0x10  0x11  0x12  0x13  0x14  0x15  0x16  0x17
    b'@', 0,    0,    0,    0,    0,    0,    0,      // q→@ (duplicate, belt+suspenders)
//  0x18  0x19  0x1A  0x1B  0x1C  0x1D  0x1E  0x1F
    0,    0,    0,    b'~', 0,    0,    0,    0,      // +=~
//  0x20  0x21  0x22  0x23  0x24  0x25  0x26  0x27
    0,    0,    0,    0,    0,    0,    0,    0,
//  0x28  0x29  0x2A  0x2B  0x2C  0x2D  0x2E  0x2F
    0,    0,    0,    0,    0,    0,    0,    0,
//  0x30  0x31  0x32  0x33  0x34  0x35  0x36  0x37
    0,    0,    0xB5, 0,    0,    0,    0,    0,      // m→µ=0xB5
//  0x38  0x39  0x3A  0x3B  0x3C  0x3D  0x3E  0x3F
    0,    0,    0,    0,    0,    0,    0,    0,
//  0x40  0x41  0x42  0x43  0x44  0x45  0x46  0x47
    0,    0,    0,    0,    0,    0,    0,    0,
//  0x48  0x49  0x4A  0x4B  0x4C  0x4D  0x4E  0x4F
    0,    0,    0,    0,    0,    0,    0,    0,
//  0x50  0x51  0x52  0x53  0x54  0x55  0x56  0x57
    0,    0,    0,    0,    0,    0,    b'|', 0,      // 0x56 extra key→|
//  0x58
    0,
];

const SC_ESC: u8 = 0x01;
const SC_LCTRL: u8 = 0x1D;
const SC_LSHIFT: u8 = 0x2A;
const SC_RSHIFT: u8 = 0x36;
const SC_LALT: u8 = 0x38;
const SC_CAPSLOCK: u8 = 0x3A;
const SC_F1: u8 = 0x3B;
const SC_F10: u8 = 0x44;
const SC_F11: u8 = 0x57;
const SC_F12: u8 = 0x58;

const SC_EXT_UP: u8 = 0x48;
const SC_EXT_DOWN: u8 = 0x50;
const SC_EXT_LEFT: u8 = 0x4B;
const SC_EXT_RIGHT: u8 = 0x4D;
const SC_EXT_HOME: u8 = 0x47;
const SC_EXT_END: u8 = 0x4F;
const SC_EXT_PGUP: u8 = 0x49;
const SC_EXT_PGDN: u8 = 0x51;
const SC_EXT_INSERT: u8 = 0x52;
const SC_EXT_DELETE: u8 = 0x53;

const BREAK_FLAG: u8 = 0x80;
const EXTENDED_PREFIX: u8 = 0xE0;

pub struct Keyboard {
    shift: bool,
    ctrl: bool,
    alt: bool,
    /// Right Alt on US, AltGr on DE.
    altgr: bool,
    caps_lock: bool,
    extended: bool,
    initialized: bool,
    layout: KeyLayout,
    /// Some controllers mis-tag kbd bytes as AUX; fallback accepts both tags.
    aux_as_kbd: bool,
}

impl Keyboard {
    /// Decoder-only construction — skips the i8042 PS/2 controller init
    /// sequence. Use when an alternative input source (e.g. USB HID) is
    /// already known to be available and probing PS/2 hardware would just
    /// produce a flood of warnings on a board without a PS/2 controller.
    /// The scan-code decoder itself works without controller init.
    pub fn new_decoder_only() -> Self {
        Self {
            shift: false,
            ctrl: false,
            alt: false,
            altgr: false,
            caps_lock: false,
            extended: false,
            initialized: false,
            layout: KeyLayout::Us,
            aux_as_kbd: false,
        }
    }

    pub fn new() -> Self {
        let mut kb = Self {
            shift: false,
            ctrl: false,
            alt: false,
            altgr: false,
            caps_lock: false,
            extended: false,
            initialized: false,
            layout: KeyLayout::Us,
            aux_as_kbd: false,
        };

        unsafe { kb.init_controller() };
        kb
    }

    pub fn set_layout(&mut self, layout: KeyLayout) {
        self.layout = layout;
    }

    pub fn layout(&self) -> KeyLayout {
        self.layout
    }

    pub fn is_shift(&self) -> bool {
        self.shift
    }
    pub fn is_ctrl(&self) -> bool {
        self.ctrl
    }
    pub fn is_alt(&self) -> bool {
        self.alt
    }

    pub fn aux_as_kbd(&self) -> bool {
        self.aux_as_kbd
    }

    unsafe fn init_controller(&mut self) {
        // Full i8042+keyboard reinit. No partial-trust path.
        morpheus_hal_x86_64::serial::log_info("INPUT", 935, "keyboard controller init begin");
        self.aux_as_kbd = false;
        self.initialized = false;

        asm_ps2_write_cmd(0xAD);
        Self::io_delay();
        asm_ps2_write_cmd(0xA7);
        Self::io_delay();
        Self::drain_all(512);

        // Known config: IRQs off, clocks on, translation off.
        asm_ps2_write_cmd(0x20);
        let mut cfg = self.wait_kbd_byte(100_000).unwrap_or(0x00);
        cfg &= !0x43;
        cfg &= !0x30; // bits are *disable* flags — clearing enables clocks
        asm_ps2_write_cmd(0x60);
        asm_ps2_write_data(cfg);
        Self::io_delay();
        Self::drain_all(128);

        asm_ps2_write_cmd(0xAA);
        let ctl_ok = self.wait_kbd_byte(200_000) == Some(0x55);
        if !ctl_ok {
            morpheus_hal_x86_64::serial::log_warn("INPUT", 936, "8042 self-test failed");
        }

        // Self-test rewrites config on some 8042s. Re-assert.
        asm_ps2_write_cmd(0x60);
        asm_ps2_write_data(cfg);
        Self::io_delay();

        asm_ps2_write_cmd(0xAB);
        let port1_ok = self.wait_kbd_byte(100_000) == Some(0x00);
        if !port1_ok {
            morpheus_hal_x86_64::serial::log_warn("INPUT", 937, "8042 port1 test failed");
        }

        asm_ps2_write_cmd(0xAE);
        Self::io_delay();
        asm_ps2_write_cmd(0xA7);
        Self::io_delay();
        Self::drain_all(128);

        let mut reset_ok = false;
        for _ in 0..3 {
            asm_ps2_write_data(0xFF);
            let ack = self.wait_kbd_byte(150_000);
            let bat = self.wait_kbd_byte(300_000);
            if ack == Some(0xFA) && bat == Some(0xAA) {
                reset_ok = true;
                break;
            }
            Self::drain_all(128);
            asm_ps2_write_cmd(0xAD);
            Self::io_delay();
            asm_ps2_write_cmd(0xAE);
            Self::io_delay();
        }
        if !reset_ok {
            morpheus_hal_x86_64::serial::log_warn(
                "INPUT",
                938,
                "keyboard reset/BAT failed after retries",
            );
        }

        // Set scan code set 1 and verify it actually latched — BIOS defaults vary.
        let mut scan_ok = true;

        asm_ps2_write_data(0xF5);
        scan_ok &= self.wait_kbd_byte(100_000) == Some(0xFA);

        asm_ps2_write_data(0xF6);
        scan_ok &= self.wait_kbd_byte(100_000) == Some(0xFA);

        asm_ps2_write_data(0xF0);
        scan_ok &= self.wait_kbd_byte(100_000) == Some(0xFA);
        asm_ps2_write_data(0x01);
        scan_ok &= self.wait_kbd_byte(100_000) == Some(0xFA);

        asm_ps2_write_data(0xF0);
        scan_ok &= self.wait_kbd_byte(100_000) == Some(0xFA);
        asm_ps2_write_data(0x00);
        scan_ok &= self.wait_kbd_byte(100_000) == Some(0xFA);
        let set_id = self.wait_kbd_byte(100_000);
        scan_ok &= set_id == Some(0x01);

        asm_ps2_write_data(0xF4);
        let f4_ack = self.wait_kbd_byte(100_000);
        scan_ok &= f4_ack == Some(0xFA);

        if !scan_ok {
            morpheus_hal_x86_64::serial::log_warn(
                "INPUT",
                931,
                "keyboard scan-set programming failed",
            );
        }

        Self::drain_all(128);

        self.initialized = ctl_ok && port1_ok && reset_ok && scan_ok;
        if self.initialized {
            morpheus_hal_x86_64::serial::log_ok(
                "INPUT",
                930,
                "PS/2 keyboard ready (full reset path)",
            );
        } else {
            morpheus_hal_x86_64::serial::log_warn(
                "INPUT",
                941,
                "PS/2 keyboard init failed (full reset path)",
            );
        }
    }

    unsafe fn wait_kbd_byte(&mut self, max_spins: u32) -> Option<u8> {
        for _ in 0..max_spins {
            let r = asm_ps2_poll_any();
            let tag = r & 0x300;
            if tag == 0x100 {
                return Some((r & 0xFF) as u8);
            }
            if tag == 0x300 {
                continue; // mouse byte during kbd bring-up; drop it
            }
            core::hint::spin_loop();
        }
        None
    }

    unsafe fn drain_all(max_reads: u32) {
        for _ in 0..max_reads {
            if asm_ps2_poll_any() == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    /// Port-0x80 dummy write — classic ~1µs ISA bus settle delay.
    #[inline(always)]
    unsafe fn io_delay() {
        core::arch::asm!(
            "out 0x80, al",
            out("al") _,
            options(nostack, preserves_flags, nomem),
        );
    }

    /// Non-blocking. `None` means empty buffer or modifier/break only.
    pub fn read_key(&mut self) -> Option<InputKey> {
        let raw = unsafe { asm_ps2_poll_any() };
        let tag = raw & 0x300;
        if tag != 0x100 && !(self.aux_as_kbd && tag == 0x300) {
            return None;
        }
        let byte = (raw & 0xFF) as u8;
        self.decode(byte)
    }

    pub fn feed_raw(&mut self, byte: u8) -> Option<InputKey> {
        self.decode(byte)
    }

    /// Blocks until a printable/actionable key arrives.
    pub fn wait_for_key(&mut self) -> InputKey {
        loop {
            let raw = unsafe { asm_ps2_poll_any() };
            let tag = raw & 0x300;
            if tag == 0x100 || (self.aux_as_kbd && tag == 0x300) {
                let byte = (raw & 0xFF) as u8;

                if let Some(key) = self.decode(byte) {
                    return key;
                }

                // Boot gate accepts any non-break make byte, even unmapped/modifier.
                if byte != EXTENDED_PREFIX && (byte & BREAK_FLAG) == 0 {
                    return InputKey {
                        scan_code: SCAN_NULL,
                        unicode_char: 0,
                    };
                }
            }

            // Interrupt wakeups aren't wired up this early — poll with backoff.
            for _ in 0..4096 {
                core::hint::spin_loop();
            }
        }
    }

    /// ~16ms-paced poll for ~60Hz animation. HLTs between RDTSC checks.
    pub fn poll_key_with_delay(&mut self) -> Option<InputKey> {
        let key = self.read_key();
        let start: u64 = unsafe {
            let lo: u32;
            let hi: u32;
            core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem));
            ((hi as u64) << 32) | (lo as u64)
        };
        let target_cycles: u64 = 16_000_000;
        loop {
            let now: u64 = unsafe {
                let lo: u32;
                let hi: u32;
                core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem));
                ((hi as u64) << 32) | (lo as u64)
            };
            if now.wrapping_sub(start) >= target_cycles {
                break;
            }
            unsafe {
                core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
            }
        }
        key
    }

    /// Returns `Some` only on a complete key-press. Modifiers and break
    /// codes return `None` after updating state.
    fn decode(&mut self, byte: u8) -> Option<InputKey> {
        if byte == EXTENDED_PREFIX {
            self.extended = true;
            return None;
        }

        let is_break = byte & BREAK_FLAG != 0;
        let make = byte & !BREAK_FLAG;

        if self.extended {
            self.extended = false;
            return self.decode_extended(make, is_break);
        }

        if is_break {
            match make {
                SC_LSHIFT | SC_RSHIFT => self.shift = false,
                SC_LCTRL => self.ctrl = false,
                SC_LALT => self.alt = false,
                _ => {},
            }
            return None;
        }

        match make {
            SC_LSHIFT | SC_RSHIFT => {
                self.shift = true;
                return None;
            },
            SC_LCTRL => {
                self.ctrl = true;
                return None;
            },
            SC_LALT => {
                self.alt = true;
                return None;
            },
            SC_CAPSLOCK => {
                self.caps_lock = !self.caps_lock;
                return None;
            },
            _ => {},
        }

        if make == SC_ESC {
            return Some(InputKey {
                scan_code: SCAN_ESC,
                unicode_char: 0,
            });
        }

        if (SC_F1..=SC_F10).contains(&make) {
            let fkey = SCAN_F1 + (make - SC_F1) as u16;
            return Some(InputKey {
                scan_code: fkey,
                unicode_char: 0,
            });
        }

        if make == SC_F11 {
            return Some(InputKey {
                scan_code: SCAN_F11,
                unicode_char: 0,
            });
        }
        if make == SC_F12 {
            return Some(InputKey {
                scan_code: SCAN_F12,
                unicode_char: 0,
            });
        }

        if (make as usize) < US_UNSHIFTED.len() {
            let ch = self.translate_ascii(make);
            if ch != 0 {
                // Ctrl+letter → 0x01..=0x1A control chars.
                let unicode = if self.ctrl && ch.is_ascii_lowercase() {
                    (ch - b'a' + 1) as u16
                } else if self.ctrl && ch.is_ascii_uppercase() {
                    (ch - b'A' + 1) as u16
                } else {
                    ch as u16
                };
                return Some(InputKey {
                    scan_code: SCAN_NULL,
                    unicode_char: unicode,
                });
            }
        }

        None
    }

    /// 0xE0-prefixed scancodes: arrows, nav, right modifiers, numpad enter/slash.
    fn decode_extended(&mut self, make: u8, is_break: bool) -> Option<InputKey> {
        if is_break {
            match make {
                SC_LCTRL => self.ctrl = false,
                SC_LALT => {
                    if self.layout == KeyLayout::De {
                        self.altgr = false;
                    } else {
                        self.alt = false;
                    }
                },
                _ => {},
            }
            return None;
        }

        match make {
            SC_LCTRL => {
                self.ctrl = true;
                return None;
            },
            SC_LALT => {
                if self.layout == KeyLayout::De {
                    self.altgr = true;
                } else {
                    self.alt = true;
                }
                return None;
            },
            _ => {},
        }

        if make == 0x1C {
            return Some(InputKey {
                scan_code: SCAN_NULL,
                unicode_char: KEY_ENTER,
            });
        }

        if make == 0x35 {
            return Some(InputKey {
                scan_code: SCAN_NULL,
                unicode_char: b'/' as u16,
            });
        }

        let scan = match make {
            SC_EXT_UP => SCAN_UP,
            SC_EXT_DOWN => SCAN_DOWN,
            SC_EXT_LEFT => SCAN_LEFT,
            SC_EXT_RIGHT => SCAN_RIGHT,
            SC_EXT_HOME => SCAN_HOME,
            SC_EXT_END => SCAN_END,
            SC_EXT_PGUP => SCAN_PGUP,
            SC_EXT_PGDN => SCAN_PGDN,
            SC_EXT_INSERT => SCAN_INSERT,
            SC_EXT_DELETE => SCAN_DELETE,
            _ => return None,
        };

        Some(InputKey {
            scan_code: scan,
            unicode_char: 0,
        })
    }

    /// Layout + shift + caps + AltGr → ASCII. 0 = no char in active layer.
    fn translate_ascii(&self, make: u8) -> u8 {
        let idx = make as usize;
        if idx >= 89 {
            return 0;
        }

        match self.layout {
            KeyLayout::Us => {
                let base = US_UNSHIFTED[idx];
                if base == 0 {
                    return 0;
                }
                let is_letter = base.is_ascii_lowercase();
                if is_letter {
                    if self.shift ^ self.caps_lock {
                        US_SHIFTED[idx]
                    } else {
                        base
                    }
                } else if self.shift {
                    US_SHIFTED[idx]
                } else {
                    base
                }
            },
            KeyLayout::De => {
                if self.altgr {
                    return DE_ALTGR[idx];
                }
                let base = DE_UNSHIFTED[idx];
                if base == 0 {
                    return 0;
                }
                // ASCII a-z respect caps; umlauts and symbols only respect shift.
                let is_ascii_lower = base.is_ascii_lowercase();
                if is_ascii_lower {
                    if self.shift ^ self.caps_lock {
                        DE_SHIFTED[idx]
                    } else {
                        base
                    }
                } else if self.shift {
                    DE_SHIFTED[idx]
                } else {
                    base
                }
            },
        }
    }
}
