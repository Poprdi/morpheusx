//! Keyboard input — PS/2 port 0x60/0x64.
//!
//! Right now this is the scaffold. The real driver will be ASM-backed
//! (port I/O via in/out) with Rust orchestration, same split as every
//! other driver in this repo. Until then: poll returns None, blocking
//! wait spins. The API is final — only the guts change.

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InputKey {
    pub scan_code: u16,
    pub unicode_char: u16,
}

// Scan codes for special keys (PS/2 set 1, same values UEFI used)
pub const SCAN_UP: u16 = 0x01;
pub const SCAN_DOWN: u16 = 0x02;
pub const SCAN_RIGHT: u16 = 0x03;
pub const SCAN_LEFT: u16 = 0x04;
pub const SCAN_ESC: u16 = 0x17;

// ASCII codes
pub const KEY_ENTER: u16 = 0x0D;
pub const KEY_SPACE: u16 = 0x20;

pub struct Keyboard {
    // Will hold PS/2 controller state once the driver lands.
    // Port 0x60 = data, 0x64 = status/command.
    _placeholder: u8,
}

impl Keyboard {
    /// Create keyboard driver. Today: empty shell. Tomorrow: PS/2 init
    /// sequence (disable scanning → self-test → enable → set scan set 1).
    pub fn new() -> Self {
        Self { _placeholder: 0 }
    }

    /// Non-blocking key read. Returns None until the PS/2 driver exists.
    pub fn read_key(&mut self) -> Option<InputKey> {
        // TODO: PS/2 driver — check port 0x64 status bit 0, read 0x60,
        // translate scan code set 1 → InputKey.
        None
    }

    /// Blocking wait. Spins until a key arrives.
    pub fn wait_for_key(&mut self) -> InputKey {
        loop {
            if let Some(key) = self.read_key() {
                return key;
            }
            for _ in 0..10_000 {
                unsafe { core::ptr::read_volatile(&0); }
            }
        }
    }

    /// Poll with ~16ms delay for animation loops (~60Hz frame pacing).
    pub fn poll_key_with_delay(&mut self) -> Option<InputKey> {
        let key = self.read_key();
        for _ in 0..100_000 {
            unsafe { core::ptr::read_volatile(&0); }
        }
        key
    }
}
