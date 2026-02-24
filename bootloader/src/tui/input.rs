//! PS/2 Keyboard Driver — full ASCII, scan code set 1, US/DE keymaps.
//!
//! Keyboard layout is a runtime value — switch with `keyboard.set_layout()`.
//! Default: `KeyLayout::Us`. German (`KeyLayout::De`) fully supported.
//!
//! Architecture mirrors every other driver in this repo:
//!   ASM primitives (bootloader/asm/keyboard/ps2.s)
//!     → Rust bindings (extern "win64")
//!       → Orchestration (init, poll, translate)
//!
//! Port 0x60 = data, port 0x64 = status/command.
//! Status bit 0 (OBF) = data ready. Bit 1 (IBF) = controller busy.
//!
//! We track shift, ctrl, alt, capslock, altgr as modifier state and produce
//! InputKey { scan_code, unicode_char } — same struct the TUI consumes.
//! scan_code uses EFI-compatible values so main_menu.rs needs zero changes.

// ASM BINDINGS — from bootloader/asm/keyboard/ps2.s

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

// PUBLIC KEY TYPE

/// Key event. scan_code != 0 for special keys, unicode_char != 0 for ASCII.
/// Both can be set simultaneously (e.g. Enter = scan_code 0, unicode_char 0x0D).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct InputKey {
    pub scan_code: u16,
    pub unicode_char: u16,
}

// efi-compatible scan codes (main_menu.rs expects these exact values)

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

// ascii codes

pub const KEY_ENTER: u16 = 0x0D;
pub const KEY_SPACE: u16 = 0x20;
pub const KEY_TAB: u16 = 0x09;
pub const KEY_BACKSPACE: u16 = 0x08;

// KEYBOARD LAYOUT

/// Active keyboard layout. Switchable at runtime via `keyboard.set_layout()`.
///
/// Affects:
///   - unshifted table (base layer)
///   - shifted table   (shift layer)
///   - altgr table     (AltGr layer — DE only, produces äöüß{[]}|@€ etc.)
///
/// US is the default. Switch to DE at any point with `set_layout(KeyLayout::De)`.
#[derive(Clone, Copy, PartialEq)]
pub enum KeyLayout {
    /// Standard US QWERTY (default)
    Us,
    /// German QWERTZ (de-DE)
    De,
}

// SCAN CODE SET 1 → ASCII TRANSLATION TABLES

// us qwerty

/// Unshifted ASCII for scan code set 1 make codes 0x00..0x58, US QWERTY.
/// 0 = no printable character (modifier or special key).
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

/// Shifted ASCII for scan code set 1 make codes 0x00..0x58, US QWERTY.
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

// de qwertz
//
// Key divergences from US:
//   0x15 (US y)  → z      0x2C (US z) → y
//   0x0C (US -)  → ß      0x0D (US =) → ' (dead accent, emit as apostrophe)
//   0x1A (US [)  → ü      0x1B (US ]) → +
//   0x27 (US ;)  → ö      0x28 (US ') → ä
//   0x29 (US `)  → ^ (dead caret, emit ^)   0x2B (US \) → #
//   0x33 (US ,)  → ,      0x34 (US .) → .   0x35 (US /) → - (dash)
//   0x56 (extra) → <   shifted → >   AltGr → |
//
// Non-ASCII: ß=0xDF ü=0xFC ö=0xF6 ä=0xE4 Ü=0xDC Ö=0xD6 Ä=0xC4 §=0xA7 µ=0xB5
// These are their Latin-1 codepoints, all fit in u8.

/// Unshifted layer for DE QWERTZ.
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

/// Shifted layer for DE QWERTZ.
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

/// AltGr (Right Alt) layer for DE QWERTZ. 0 = no mapping.
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

// PS/2 SCAN CODE CONSTANTS

// Make codes (key press)
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

// Extended (0xE0 prefix) make codes
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

// Break code flag
const BREAK_FLAG: u8 = 0x80;

// Extended prefix byte
const EXTENDED_PREFIX: u8 = 0xE0;

// KEYBOARD STATE

pub struct Keyboard {
    /// Shift held (left or right)
    shift: bool,
    /// Ctrl held
    ctrl: bool,
    /// Alt held (left only — right alt = AltGr on DE)
    alt: bool,
    /// AltGr held (right alt, or Ctrl+LAlt on some firmware)
    altgr: bool,
    /// Caps lock toggled
    caps_lock: bool,
    /// Next byte is an extended (0xE0) scancode
    extended: bool,
    /// Controller initialized
    initialized: bool,
    /// Active keymap
    layout: KeyLayout,
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
    ///
    /// Initialize the PS/2 keyboard controller with the default US layout.
    /// Call `set_layout(KeyLayout::De)` at any point to switch to German.
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
        };

        unsafe { kb.init_controller() };
        kb
    }

    /// Switch keyboard layout at runtime. Takes effect immediately.
    pub fn set_layout(&mut self, layout: KeyLayout) {
        self.layout = layout;
    }

    /// Return the currently active layout.
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

    unsafe fn init_controller(&mut self) {
        use morpheus_hwinit::serial::puts;

        // OVMF has already configured the 8042 correctly — we must NOT send
        // 0xAA (controller self-test) or 0xFF (keyboard reset) here.
        //
        // 0xAA resets the Configuration Byte, which clears bit 6 (Translation).
        // Translation is what makes the 8042 convert scan-code-set-2 bytes
        // (what the keyboard natively sends) into set-1 (what our tables
        // decode). If translation is cleared and we then fail to set native
        // set-1 on the keyboard, raw set-2 bytes arrive and decode completely
        // wrong — every key produces garbage.
        //
        // Strategy: read-modify-write the config byte so we preserve OVMF's
        // state, guarantee translation is on, and disable IRQs (we poll).
        // Then enable scanning. That's it.

        // 1. Flush stale bytes from OVMF
        asm_ps2_flush();

        // 2. Disable port 1 scanning while we touch the config
        asm_ps2_write_cmd(0xAD);
        Self::io_delay();
        asm_ps2_flush();

        // 3. Read current Configuration Byte (cmd 0x20 → byte on data port)
        asm_ps2_write_cmd(0x20);
        let config = Self::wait_response(50_000).unwrap_or(0x45);
        // Bit layout:
        //   bit 0 — port-1 IRQ enabled   (we clear: polling only)
        //   bit 1 — port-2 IRQ enabled   (we clear)
        //   bit 4 — port-1 clock disable (leave as-is)
        //   bit 5 — port-2 clock disable (leave as-is)
        //   bit 6 — Translation enabled  (we SET: 8042 converts set-2 → set-1)
        let new_config = (config | 0x40) & !0x03;

        // 4. Write modified config byte (cmd 0x60, then data byte)
        asm_ps2_write_cmd(0x60);
        asm_ps2_write_data(new_config);
        Self::io_delay();

        // 5. Re-enable port 1
        asm_ps2_write_cmd(0xAE);
        Self::io_delay();

        // 6. Tell the keyboard to enable scanning (0xF4 → expects 0xFA ACK)
        //    This is the ONLY command we send to the keyboard device itself.
        asm_ps2_write_data(0xF4);
        let _ = Self::wait_response(50_000);

        // 7. Final flush — discard any ACK noise
        asm_ps2_flush();

        self.initialized = true;
        puts("[KBD] PS/2 keyboard ready (8042 translation, set-1 tables)\n");
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

    // PUBLIC API (unchanged signatures — TUI code needs zero changes)

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

    // SCAN CODE DECODER

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

        // break codes (key release) — update modifiers, emit nothing
        if is_break {
            match make {
                SC_LSHIFT | SC_RSHIFT => self.shift = false,
                SC_LCTRL => self.ctrl = false,
                SC_LALT => self.alt = false,
                _ => {} // ignore other releases
            }
            return None;
        }

        // make codes (key press)

        // Modifiers
        match make {
            SC_LSHIFT | SC_RSHIFT => {
                self.shift = true;
                return None;
            }
            SC_LCTRL => {
                self.ctrl = true;
                return None;
            }
            SC_LALT => {
                self.alt = true;
                return None;
            }
            SC_CAPSLOCK => {
                self.caps_lock = !self.caps_lock;
                return None;
            }
            _ => {}
        }

        // Escape
        if make == SC_ESC {
            return Some(InputKey {
                scan_code: SCAN_ESC,
                unicode_char: 0,
            });
        }

        // Function keys F1-F10
        if (SC_F1..=SC_F10).contains(&make) {
            let fkey = SCAN_F1 + (make - SC_F1) as u16;
            return Some(InputKey {
                scan_code: fkey,
                unicode_char: 0,
            });
        }

        // F11, F12
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

        // ASCII from translation table
        if (make as usize) < US_UNSHIFTED.len() {
            let ch = self.translate_ascii(make);
            if ch != 0 {
                // Ctrl+letter produces control characters (0x01-0x1A)
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

        None // unrecognized or non-printable
    }

    /// Decode an extended (0xE0-prefixed) scancode.
    fn decode_extended(&mut self, make: u8, is_break: bool) -> Option<InputKey> {
        // Extended modifier releases (right ctrl, right alt)
        if is_break {
            match make {
                SC_LCTRL => self.ctrl = false, // Right Ctrl shares make code
                SC_LALT => {
                    // Right Alt = AltGr on DE; plain Alt on US
                    if self.layout == KeyLayout::De {
                        self.altgr = false;
                    } else {
                        self.alt = false;
                    }
                }
                _ => {}
            }
            return None;
        }

        // Extended modifier presses
        match make {
            SC_LCTRL => {
                self.ctrl = true;
                return None;
            }
            SC_LALT => {
                if self.layout == KeyLayout::De {
                    self.altgr = true;
                } else {
                    self.alt = true;
                }
                return None;
            }
            _ => {}
        }

        // Navigation keys → EFI scan codes
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

    /// Look up a character for the given make code, applying the active
    /// layout, shift, caps lock, and (for DE) AltGr.
    ///
    /// Returns 0 if the key has no character in the active layer.
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
            }
            KeyLayout::De => {
                // AltGr layer takes priority over everything else
                if self.altgr {
                    let ch = DE_ALTGR[idx];
                    return ch; // 0 = no AltGr mapping for this key
                }
                let base = DE_UNSHIFTED[idx];
                if base == 0 {
                    return 0;
                }
                // Letters are still ASCII a-z in DE table; umlauts are not letters
                let is_ascii_lower = base.is_ascii_lowercase();
                if is_ascii_lower {
                    if self.shift ^ self.caps_lock {
                        DE_SHIFTED[idx]
                    } else {
                        base
                    }
                } else {
                    // Non-letter (punctuation, umlauts, symbols): shift only
                    if self.shift {
                        DE_SHIFTED[idx]
                    } else {
                        base
                    }
                }
            }
        }
    }
}
