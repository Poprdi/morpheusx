use crate::SimpleTextInputProtocol;

#[repr(C)]
pub struct InputKey {
    pub scan_code: u16,
    pub unicode_char: u16,
}

// Scan codes for special keys
pub const SCAN_UP: u16 = 0x01;
pub const SCAN_DOWN: u16 = 0x02;
pub const SCAN_RIGHT: u16 = 0x03;
pub const SCAN_LEFT: u16 = 0x04;
pub const SCAN_ESC: u16 = 0x17;

// ASCII codes
pub const KEY_ENTER: u16 = 0x0D;
pub const KEY_SPACE: u16 = 0x20;

pub struct Keyboard {
    input: *mut SimpleTextInputProtocol,
}

impl Keyboard {
    pub fn new(input: *mut SimpleTextInputProtocol) -> Self {
        Self { input }
    }

    pub fn read_key(&mut self) -> Option<InputKey> {
        unsafe {
            let mut key = InputKey {
                scan_code: 0,
                unicode_char: 0,
            };

            let status = ((*self.input).read_key_stroke)(self.input, &mut key);
            
            // EFI_SUCCESS = 0
            if status == 0 {
                Some(key)
            } else {
                None
            }
        }
    }

    pub fn wait_for_key(&mut self) -> InputKey {
        loop {
            if let Some(key) = self.read_key() {
                return key;
            }
            // Small delay to avoid spinning
            for _ in 0..10000 {
                unsafe { core::ptr::read_volatile(&0); }
            }
        }
    }
    
    // Poll for key with a small delay for animation loops (approx 60Hz)
    pub fn poll_key_with_delay(&mut self) -> Option<InputKey> {
        let key = self.read_key();
        
        // Minimal delay for responsive input
        for _ in 0..10000 {
            unsafe { core::ptr::read_volatile(&0); }
        }
        
        key
    }
}
