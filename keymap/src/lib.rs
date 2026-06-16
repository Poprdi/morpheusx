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

#![cfg_attr(not(test), no_std)]

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


// PS/2 Set-1 make codes for the modifiers the batch decoder tracks.
const SC_LSHIFT: u8 = 0x2a;
const SC_RSHIFT: u8 = 0x36;
const SC_CTRL: u8 = 0x1d; // left Ctrl; right Ctrl arrives 0xE0-prefixed with the same code
const SC_ALT: u8 = 0x38; // left Alt; right Alt (AltGr) arrives 0xE0-prefixed
const SC_CAPS: u8 = 0x3a;

/// One resolved keystroke, handed to [`ScanDecoder::feed_batch`]'s callback on a key's
/// **release** edge. `bytes` is what the active layout produced (a UTF-8 character, a C0
/// control, or a `CSI` sequence for the navigation cluster) — empty for an unmapped key.
/// `scancode` and the modifier flags let the caller intercept its own chord hotkeys before
/// forwarding `bytes`.
pub struct Key<'a> {
    pub scancode: u8,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub bytes: &'a [u8],
}

/// Stateful PS/2 Set-1 scancode → terminal-bytes decoder.
///
/// It acts on the **release** edge, not the press edge — deliberately. On a real QEMU+OVMF
/// MorpheusX boot the kernel's PS/2 path delivers *break* (release) codes intact but corrupts
/// *make* (press) codes (e.g. `a`'s make `0x1E` arrives as `0x03`, and a corrupted make
/// sometimes lands on a modifier scancode — `t` → `1d` looks like Ctrl). A break is exactly
/// `make | 0x80` and arrives clean, so recovering the true key from `byte & 0x7F` on release
/// sidesteps the corruption entirely; it is equally correct on clean-make hardware (emitting on
/// release is simply consistent). Modifiers are taken from persistent state OR a same-batch
/// look-ahead to the modifier's clean break, and a bare modifier *make* only sets persistent
/// state when the batch releases no real key (otherwise it is a corrupted key-make in disguise).
///
/// Feed it whole `SYS_KEYBOARD_READ` bursts: the look-ahead needs the `make,make,break,break`
/// of a chord (`:`, `?`, Ctrl-C) in one batch, so coalesce a key burst before calling.
#[derive(Default)]
pub struct ScanDecoder {
    shift: bool,
    ctrl: bool,
    alt: bool,
    altgr: bool,
    caps: bool,
    extended: bool,
}

impl ScanDecoder {
    pub const fn new() -> Self {
        ScanDecoder {
            shift: false,
            ctrl: false,
            alt: false,
            altgr: false,
            caps: false,
            extended: false,
        }
    }

    /// Decode one coalesced scancode batch against `km`, invoking `on_key` once per resolved
    /// key release with the decoded bytes and effective modifier state.
    pub fn feed_batch(&mut self, km: &Keymap, scan: &[u8], mut on_key: impl FnMut(Key<'_>)) {
        let has_key_release = batch_has_key_release(scan);
        for i in 0..scan.len() {
            let byte = scan[i];
            if byte == 0xe0 {
                self.extended = true;
                continue;
            }
            let is_break = byte & 0x80 != 0;
            let make = byte & 0x7f;
            let ext = self.extended;
            self.extended = false;

            if ext {
                // 0xE0-prefixed: right-hand Ctrl / AltGr, and the navigation cluster.
                match make {
                    SC_CTRL => set_mod(&mut self.ctrl, is_break, has_key_release),
                    SC_ALT => set_mod(&mut self.altgr, is_break, has_key_release),
                    _ => {
                        if is_break {
                            if let Some(seq) = ext_nav_seq(make) {
                                on_key(Key {
                                    scancode: make,
                                    ctrl: self.ctrl,
                                    alt: self.alt,
                                    shift: self.shift,
                                    bytes: seq,
                                });
                            }
                        }
                    },
                }
                continue;
            }

            // Modifiers / locks: a clean break always releases; a make sets persistent state
            // only when it cannot be a corrupted key-make (no real key released this batch).
            match make {
                SC_LSHIFT | SC_RSHIFT => {
                    set_mod(&mut self.shift, is_break, has_key_release);
                    continue;
                },
                SC_CTRL => {
                    set_mod(&mut self.ctrl, is_break, has_key_release);
                    continue;
                },
                SC_ALT => {
                    set_mod(&mut self.alt, is_break, has_key_release);
                    continue;
                },
                SC_CAPS => {
                    if !is_break && !has_key_release {
                        self.caps = !self.caps;
                    }
                    continue;
                },
                _ => {},
            }

            // Non-modifier presses carry the corrupted scancode — ignore; act on the break.
            if !is_break {
                continue;
            }

            // A key release: `make` is the true scancode. Effective modifiers = persistent
            // state OR this modifier's clean break appearing later in the batch (covers the
            // corrupted-make chord where the modifier's own press was never recognized).
            let shift = self.shift || break_ahead(scan, i, &[SC_LSHIFT, SC_RSHIFT]);
            let ctrl = self.ctrl || break_ahead(scan, i, &[SC_CTRL]);
            let alt = self.alt || break_ahead(scan, i, &[SC_ALT]);
            let mods = Mods {
                shift,
                altgr: self.altgr,
                ctrl,
                caps: self.caps,
            };
            let mut out = [0u8; 4];
            let n = km.decode(make, &mods, &mut out);
            on_key(Key {
                scancode: make,
                ctrl,
                alt,
                shift,
                bytes: &out[..n],
            });
        }
    }
}

/// Apply a modifier scancode edge: a clean break clears; a make sets only when the batch
/// released no real key (else the make is a corrupted key-make masquerading as this modifier).
#[inline]
fn set_mod(flag: &mut bool, is_break: bool, has_key_release: bool) {
    if is_break {
        *flag = false;
    } else if !has_key_release {
        *flag = true;
    }
}

/// Does a clean break of any scancode in `mods` appear in `scan[i+1..]`?
fn break_ahead(scan: &[u8], i: usize, mods: &[u8]) -> bool {
    scan[i + 1..]
        .iter()
        .any(|&b| b & 0x80 != 0 && mods.contains(&(b & 0x7f)))
}

/// True if this batch releases at least one *non-modifier* key. Such a break means a real key
/// was pressed in the batch, so a bare modifier *make* riding alongside it is a corrupted
/// key-make, not a held modifier. The `0xE0` prefix is skipped so an arrow's `e0 d0` still
/// counts as a (non-modifier) release.
fn batch_has_key_release(scan: &[u8]) -> bool {
    let mods = [SC_LSHIFT, SC_RSHIFT, SC_CTRL, SC_ALT];
    scan.iter()
        .any(|&b| b != 0xe0 && b & 0x80 != 0 && !mods.contains(&(b & 0x7f)))
}

/// Terminal escape sequence for a 0xE0-prefixed navigation key (by its make code), or `None`.
fn ext_nav_seq(make: u8) -> Option<&'static [u8]> {
    Some(match make {
        0x48 => b"\x1b[A",  // Up
        0x50 => b"\x1b[B",  // Down
        0x4d => b"\x1b[C",  // Right
        0x4b => b"\x1b[D",  // Left
        0x47 => b"\x1b[H",  // Home
        0x4f => b"\x1b[F",  // End
        0x49 => b"\x1b[5~", // PageUp
        0x51 => b"\x1b[6~", // PageDown
        0x53 => b"\x1b[3~", // Delete
        _ => return None,
    })
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

#[cfg(test)]
mod scan_tests {
    use super::*;

    /// Run a US-layout batch and collect the forwarded bytes (hotkey chords pass through too).
    fn run(dec: &mut ScanDecoder, scan: &[u8]) -> Vec<u8> {
        let km = us_default();
        let mut acc = Vec::new();
        dec.feed_batch(&km, scan, |k| acc.extend_from_slice(k.bytes));
        acc
    }

    #[test]
    fn decodes_on_release_not_press() {
        let mut d = ScanDecoder::new();
        assert_eq!(run(&mut d, &[0x1e]), b""); // 'a' press: nothing yet
        assert_eq!(run(&mut d, &[0x9e]), b"a"); // 'a' release: emit
    }

    #[test]
    fn corrupted_make_ignored_break_is_truth() {
        let mut d = ScanDecoder::new();
        // Captured QEMU traces: 'a' = corrupted make 0x03 + clean break 0x9e; Enter (→ '\n').
        assert_eq!(run(&mut d, &[0x03, 0x9e]), b"a");
        assert_eq!(run(&mut d, &[0x1e, 0x9c]), b"\n");
    }

    #[test]
    fn shift_chord_via_break_lookahead() {
        let mut d = ScanDecoder::new();
        // ':' = Shift+';'. Burst with corrupted makes + clean breaks of ';' (0xa7) and Shift
        // (0xaa); the layout maps shifted 0x27 → ':'.
        assert_eq!(run(&mut d, &[0x2f, 0x5c, 0xa7, 0xaa]), b":");
        // '?' = Shift+'/' (US 0x35). ';'/'/' breaks 0xb5, Shift break 0xaa.
        assert_eq!(run(&mut d, &[0x2f, 0x15, 0xb5, 0xaa]), b"?");
    }

    #[test]
    fn slash_unshifted() {
        let mut d = ScanDecoder::new();
        assert_eq!(run(&mut d, &[0xb5]), b"/"); // '/' release (US 0x35), no shift
    }

    #[test]
    fn corrupted_make_on_modifier_is_not_phantom_modifier() {
        // The real defect: a key's corrupted make lands on a modifier scancode (`t` → `1d`
        // = Ctrl, `e` → `2a` = LShift). The clean break is truth; the modifier make is ignored
        // because the batch releases a real key.
        let mut d = ScanDecoder::new();
        assert_eq!(run(&mut d, &[0x1d, 0x94]), b"t"); // NOT Ctrl-T
        assert_eq!(run(&mut d, &[0x2a, 0x92]), b"e"); // NOT 'E', no sticky shift…
        assert_eq!(run(&mut d, &[0x9e]), b"a"); // …next plain key stays lowercase
    }

    #[test]
    fn ctrl_c_via_break_lookahead() {
        let mut d = ScanDecoder::new();
        // Captured Ctrl-C burst 11 06 ae 9d → C0 0x03 via the layout's ctrl_transform.
        assert_eq!(run(&mut d, &[0x11, 0x06, 0xae, 0x9d]), &[0x03]);
    }

    #[test]
    fn held_modifier_clean_make_real_hardware() {
        let mut d = ScanDecoder::new();
        // Clean-make path: a lone Shift make in its own batch sets persistent state.
        assert_eq!(run(&mut d, &[SC_LSHIFT]), b""); // shift down
        assert_eq!(run(&mut d, &[0x27, 0xa7]), b":"); // ';' shifted → ':'
        assert_eq!(run(&mut d, &[0xaa]), b""); // shift up
        assert_eq!(run(&mut d, &[0x27, 0xa7]), b";"); // now unshifted
    }

    #[test]
    fn extended_arrows_on_release() {
        let mut d = ScanDecoder::new();
        assert_eq!(run(&mut d, &[0xe0, 0x48, 0xe0, 0xc8]), b"\x1b[A"); // Up
        assert_eq!(run(&mut d, &[0xe0, 0x50, 0xe0, 0xd0]), b"\x1b[B"); // Down
        assert_eq!(run(&mut d, &[0xe0, 0xcb]), b"\x1b[D"); // Left (break only)
    }

    #[test]
    fn caps_only_affects_letters() {
        let mut d = ScanDecoder::new();
        run(&mut d, &[0x3a]); // caps make (own batch → toggles)
        assert_eq!(run(&mut d, &[0x9e]), b"A"); // letter uppercased
        assert_eq!(run(&mut d, &[0x82]), b"1"); // digit unaffected
    }

    #[test]
    fn hotkey_chord_reports_scancode_and_mods() {
        // Ctrl+] : the callback sees scancode 0x1b with ctrl set so the caller can consume it.
        let mut d = ScanDecoder::new();
        let km = us_default();
        let mut seen = None;
        d.feed_batch(&km, &[0x1d, 0x5c, 0x9b, 0x9d], |k| {
            seen = Some((k.scancode, k.ctrl));
        });
        assert_eq!(seen, Some((0x1b, true)));
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
