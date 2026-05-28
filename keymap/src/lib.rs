//! Keyboard layout (`.kmap`) format, decode engine, and built-in tables.
//!
//! Shared single source of truth for:
//!   * `compd` — loads the active layout and decodes scancodes → UTF-8 bytes,
//!   * `keymap-gen` (host tool) — serializes the built-in tables to `.kmap`
//!     files that get provisioned into the HelixFS image,
//!   * a future graphical layout editor — reads/writes the same format.
//!
//! ## `.kmap` binary format (little-endian, fixed 2080 bytes)
//! ```text
//!   [0..4)    magic b"KMAP"
//!   [4..6)    version u16 = 1
//!   [6..8)    flags u16          bit0 = CapsLock affects letters
//!   [8..32)   name  [u8;24]      nul-padded UTF-8, e.g. "German (DE)"
//!   [32..)    entries[128]       index = PS/2 Set 1 make scancode (0x00..0x7F)
//!             entry = { base:u32, shift:u32, altgr:u32, shift_altgr:u32 }
//!                                UTF-32 codepoints; 0 = no output
//! ```

#![no_std]

pub const KMAP_MAGIC: [u8; 4] = *b"KMAP";
pub const KMAP_VERSION: u16 = 1;
pub const KMAP_FLAG_CAPS_LETTERS: u16 = 1 << 0;

const NUM_SCANCODES: usize = 128;
const ENTRY_BYTES: usize = 16;
const HEADER_BYTES: usize = 32;
pub const KMAP_FILE_SIZE: usize = HEADER_BYTES + NUM_SCANCODES * ENTRY_BYTES; // 2080

#[derive(Clone, Copy)]
struct Entry {
    base: u32,
    shift: u32,
    altgr: u32,
    shift_altgr: u32,
}

impl Entry {
    const fn none() -> Self {
        Self {
            base: 0,
            shift: 0,
            altgr: 0,
            shift_altgr: 0,
        }
    }
    /// Control/whitespace key — same codepoint regardless of Shift.
    const fn ch(cp: u32) -> Self {
        Self {
            base: cp,
            shift: cp,
            altgr: 0,
            shift_altgr: 0,
        }
    }
    const fn bs(base: char, shift: char) -> Self {
        Self {
            base: base as u32,
            shift: shift as u32,
            altgr: 0,
            shift_altgr: 0,
        }
    }
    const fn bsa(base: char, shift: char, altgr: char) -> Self {
        Self {
            base: base as u32,
            shift: shift as u32,
            altgr: altgr as u32,
            shift_altgr: 0,
        }
    }
}

/// Live modifier state passed from the input layer.
pub struct Mods {
    pub shift: bool,
    pub altgr: bool,
    pub ctrl: bool,
    pub caps: bool,
}

pub struct Keymap {
    flags: u16,
    entries: [Entry; NUM_SCANCODES],
}

impl Keymap {
    /// Parse a `.kmap` blob. Returns `None` on bad magic / version / short data.
    pub fn parse(data: &[u8]) -> Option<Keymap> {
        if data.len() < KMAP_FILE_SIZE || data[0..4] != KMAP_MAGIC {
            return None;
        }
        if u16::from_le_bytes([data[4], data[5]]) != KMAP_VERSION {
            return None;
        }
        let flags = u16::from_le_bytes([data[6], data[7]]);
        let mut entries = [Entry::none(); NUM_SCANCODES];
        let rd = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
        for (i, e) in entries.iter_mut().enumerate() {
            let off = HEADER_BYTES + i * ENTRY_BYTES;
            e.base = rd(off);
            e.shift = rd(off + 4);
            e.altgr = rd(off + 8);
            e.shift_altgr = rd(off + 12);
        }
        Some(Keymap { flags, entries })
    }

    /// Serialize to the fixed-size `.kmap` byte layout. `name` is truncated to
    /// 24 bytes. Used by the host generator.
    pub fn serialize(&self, name: &str, out: &mut [u8; KMAP_FILE_SIZE]) {
        for b in out.iter_mut() {
            *b = 0;
        }
        out[0..4].copy_from_slice(&KMAP_MAGIC);
        out[4..6].copy_from_slice(&KMAP_VERSION.to_le_bytes());
        out[6..8].copy_from_slice(&self.flags.to_le_bytes());
        let nb = name.as_bytes();
        let n = if nb.len() > 24 { 24 } else { nb.len() };
        out[8..8 + n].copy_from_slice(&nb[..n]);
        for (i, e) in self.entries.iter().enumerate() {
            let off = HEADER_BYTES + i * ENTRY_BYTES;
            out[off..off + 4].copy_from_slice(&e.base.to_le_bytes());
            out[off + 4..off + 8].copy_from_slice(&e.shift.to_le_bytes());
            out[off + 8..off + 12].copy_from_slice(&e.altgr.to_le_bytes());
            out[off + 12..off + 16].copy_from_slice(&e.shift_altgr.to_le_bytes());
        }
    }

    /// Decode a make scancode (`0x00..0x7F`) + modifiers into the UTF-8 bytes
    /// to forward, written into `out`. Returns the byte count (0 = no output).
    pub fn decode(&self, scancode: u8, mods: &Mods, out: &mut [u8; 4]) -> usize {
        let idx = scancode as usize;
        if idx >= NUM_SCANCODES {
            return 0;
        }
        let e = self.entries[idx];
        if e.base == 0 && e.shift == 0 && e.altgr == 0 {
            return 0; // unmapped
        }

        let is_letter = is_ascii_letter(e.base);
        let caps_letters = self.flags & KMAP_FLAG_CAPS_LETTERS != 0;
        let eff_shift = mods.shift ^ (mods.caps && caps_letters && is_letter);

        let mut cp = if mods.altgr {
            if eff_shift && e.shift_altgr != 0 {
                e.shift_altgr
            } else {
                e.altgr
            }
        } else if eff_shift {
            if e.shift != 0 {
                e.shift
            } else if is_letter {
                e.base.wrapping_sub(0x20) // uppercase a–z
            } else {
                e.base
            }
        } else {
            e.base
        };
        if cp == 0 {
            cp = e.base;
        }
        if cp == 0 {
            return 0;
        }

        // Ctrl collapses letters / @[\]^_ to control codes (Ctrl+C → 0x03 etc.).
        if mods.ctrl {
            if let Some(ctrl_byte) = ctrl_transform(cp) {
                out[0] = ctrl_byte;
                return 1;
            }
        }

        encode_utf8(cp, out)
    }
}

#[inline]
fn is_ascii_letter(cp: u32) -> bool {
    (b'a' as u32..=b'z' as u32).contains(&cp) || (b'A' as u32..=b'Z' as u32).contains(&cp)
}

/// Standard control-code mapping: uppercase the letter, mask 0x1F. Applies to
/// `@ A..Z [ \ ] ^ _`. Returns `None` for keys with no control code.
fn ctrl_transform(cp: u32) -> Option<u8> {
    let upper = if (b'a' as u32..=b'z' as u32).contains(&cp) {
        cp - 0x20
    } else {
        cp
    };
    if (0x40..=0x5F).contains(&upper) {
        Some((upper & 0x1F) as u8)
    } else {
        None
    }
}

fn encode_utf8(cp: u32, out: &mut [u8; 4]) -> usize {
    match char::from_u32(cp) {
        Some(c) => c.encode_utf8(out).len(),
        None => 0,
    }
}

/// Shared control/whitespace keys (same on every Latin layout).
fn fill_common(e: &mut [Entry; NUM_SCANCODES]) {
    e[0x01] = Entry::ch(0x1B); // Esc
    e[0x0E] = Entry::ch(0x08); // Backspace
    e[0x0F] = Entry::ch(0x09); // Tab
    e[0x1C] = Entry::ch(0x0A); // Enter → '\n'
    e[0x39] = Entry::ch(0x20); // Space
}

/// Built-in German QWERTZ — fallback when no `.kmap` file is on the FS, and the
/// source `keymap-gen` serializes to `de.kmap`.
pub fn german_default() -> Keymap {
    let mut e = [Entry::none(); NUM_SCANCODES];
    fill_common(&mut e);

    // Number row.
    e[0x29] = Entry::bs('^', '°');
    e[0x02] = Entry::bs('1', '!');
    e[0x03] = Entry::bsa('2', '"', '²');
    e[0x04] = Entry::bsa('3', '§', '³');
    e[0x05] = Entry::bs('4', '$');
    e[0x06] = Entry::bs('5', '%');
    e[0x07] = Entry::bs('6', '&');
    e[0x08] = Entry::bsa('7', '/', '{');
    e[0x09] = Entry::bsa('8', '(', '[');
    e[0x0A] = Entry::bsa('9', ')', ']');
    e[0x0B] = Entry::bsa('0', '=', '}');
    e[0x0C] = Entry::bsa('ß', '?', '\\');
    e[0x0D] = Entry::bs('´', '`');

    // QWERTZ row.
    e[0x10] = Entry::bsa('q', 'Q', '@');
    e[0x11] = Entry::bs('w', 'W');
    e[0x12] = Entry::bsa('e', 'E', '€');
    e[0x13] = Entry::bs('r', 'R');
    e[0x14] = Entry::bs('t', 'T');
    e[0x15] = Entry::bs('z', 'Z'); // US-Y position
    e[0x16] = Entry::bs('u', 'U');
    e[0x17] = Entry::bs('i', 'I');
    e[0x18] = Entry::bs('o', 'O');
    e[0x19] = Entry::bs('p', 'P');
    e[0x1A] = Entry::bs('ü', 'Ü');
    e[0x1B] = Entry::bsa('+', '*', '~');
    e[0x2B] = Entry::bs('#', '\'');

    // Home row.
    e[0x1E] = Entry::bs('a', 'A');
    e[0x1F] = Entry::bs('s', 'S');
    e[0x20] = Entry::bs('d', 'D');
    e[0x21] = Entry::bs('f', 'F');
    e[0x22] = Entry::bs('g', 'G');
    e[0x23] = Entry::bs('h', 'H');
    e[0x24] = Entry::bs('j', 'J');
    e[0x25] = Entry::bs('k', 'K');
    e[0x26] = Entry::bs('l', 'L');
    e[0x27] = Entry::bs('ö', 'Ö');
    e[0x28] = Entry::bs('ä', 'Ä');

    // Bottom row.
    e[0x56] = Entry::bsa('<', '>', '|'); // extra ISO/DE key
    e[0x2C] = Entry::bs('y', 'Y'); // US-Z position
    e[0x2D] = Entry::bs('x', 'X');
    e[0x2E] = Entry::bs('c', 'C');
    e[0x2F] = Entry::bs('v', 'V');
    e[0x30] = Entry::bs('b', 'B');
    e[0x31] = Entry::bs('n', 'N');
    e[0x32] = Entry::bsa('m', 'M', 'µ');
    e[0x33] = Entry::bs(',', ';');
    e[0x34] = Entry::bs('.', ':');
    e[0x35] = Entry::bs('-', '_');

    Keymap {
        flags: KMAP_FLAG_CAPS_LETTERS,
        entries: e,
    }
}

/// Built-in US QWERTY — serialized to `us.kmap`.
pub fn us_default() -> Keymap {
    let mut e = [Entry::none(); NUM_SCANCODES];
    fill_common(&mut e);

    // Number row.
    e[0x29] = Entry::bs('`', '~');
    e[0x02] = Entry::bs('1', '!');
    e[0x03] = Entry::bs('2', '@');
    e[0x04] = Entry::bs('3', '#');
    e[0x05] = Entry::bs('4', '$');
    e[0x06] = Entry::bs('5', '%');
    e[0x07] = Entry::bs('6', '^');
    e[0x08] = Entry::bs('7', '&');
    e[0x09] = Entry::bs('8', '*');
    e[0x0A] = Entry::bs('9', '(');
    e[0x0B] = Entry::bs('0', ')');
    e[0x0C] = Entry::bs('-', '_');
    e[0x0D] = Entry::bs('=', '+');

    // QWERTY row.
    e[0x10] = Entry::bs('q', 'Q');
    e[0x11] = Entry::bs('w', 'W');
    e[0x12] = Entry::bs('e', 'E');
    e[0x13] = Entry::bs('r', 'R');
    e[0x14] = Entry::bs('t', 'T');
    e[0x15] = Entry::bs('y', 'Y');
    e[0x16] = Entry::bs('u', 'U');
    e[0x17] = Entry::bs('i', 'I');
    e[0x18] = Entry::bs('o', 'O');
    e[0x19] = Entry::bs('p', 'P');
    e[0x1A] = Entry::bs('[', '{');
    e[0x1B] = Entry::bs(']', '}');
    e[0x2B] = Entry::bs('\\', '|');

    // Home row.
    e[0x1E] = Entry::bs('a', 'A');
    e[0x1F] = Entry::bs('s', 'S');
    e[0x20] = Entry::bs('d', 'D');
    e[0x21] = Entry::bs('f', 'F');
    e[0x22] = Entry::bs('g', 'G');
    e[0x23] = Entry::bs('h', 'H');
    e[0x24] = Entry::bs('j', 'J');
    e[0x25] = Entry::bs('k', 'K');
    e[0x26] = Entry::bs('l', 'L');
    e[0x27] = Entry::bs(';', ':');
    e[0x28] = Entry::bs('\'', '"');

    // Bottom row.
    e[0x2C] = Entry::bs('z', 'Z');
    e[0x2D] = Entry::bs('x', 'X');
    e[0x2E] = Entry::bs('c', 'C');
    e[0x2F] = Entry::bs('v', 'V');
    e[0x30] = Entry::bs('b', 'B');
    e[0x31] = Entry::bs('n', 'N');
    e[0x32] = Entry::bs('m', 'M');
    e[0x33] = Entry::bs(',', '<');
    e[0x34] = Entry::bs('.', '>');
    e[0x35] = Entry::bs('/', '?');

    Keymap {
        flags: KMAP_FLAG_CAPS_LETTERS,
        entries: e,
    }
}
