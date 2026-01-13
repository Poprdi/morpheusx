//! EFI text attribute to RGB color conversion.
//!
//! EFI text attributes: foreground bits 0-3, background bits 4-6.

use crate::types::Color;

/// EFI text color indices.
pub mod efi {
    pub const BLACK: u8 = 0x00;
    pub const BLUE: u8 = 0x01;
    pub const GREEN: u8 = 0x02;
    pub const CYAN: u8 = 0x03;
    pub const RED: u8 = 0x04;
    pub const MAGENTA: u8 = 0x05;
    pub const BROWN: u8 = 0x06;
    pub const LIGHTGRAY: u8 = 0x07;
    pub const DARKGRAY: u8 = 0x08;
    pub const LIGHTBLUE: u8 = 0x09;
    pub const LIGHTGREEN: u8 = 0x0A;
    pub const LIGHTCYAN: u8 = 0x0B;
    pub const LIGHTRED: u8 = 0x0C;
    pub const LIGHTMAGENTA: u8 = 0x0D;
    pub const YELLOW: u8 = 0x0E;
    pub const WHITE: u8 = 0x0F;

    /// Default attribute: light gray on black.
    pub const DEFAULT_ATTR: u8 = LIGHTGRAY;
}

/// Standard VGA/EFI 16-color palette.
const EFI_PALETTE: [Color; 16] = [
    Color::rgb(0, 0, 0),       // 0: Black
    Color::rgb(0, 0, 170),     // 1: Blue
    Color::rgb(0, 170, 0),     // 2: Green
    Color::rgb(0, 170, 170),   // 3: Cyan
    Color::rgb(170, 0, 0),     // 4: Red
    Color::rgb(170, 0, 170),   // 5: Magenta
    Color::rgb(170, 85, 0),    // 6: Brown
    Color::rgb(170, 170, 170), // 7: Light Gray
    Color::rgb(85, 85, 85),    // 8: Dark Gray
    Color::rgb(85, 85, 255),   // 9: Light Blue
    Color::rgb(85, 255, 85),   // A: Light Green
    Color::rgb(85, 255, 255),  // B: Light Cyan
    Color::rgb(255, 85, 85),   // C: Light Red
    Color::rgb(255, 85, 255),  // D: Light Magenta
    Color::rgb(255, 255, 85),  // E: Yellow
    Color::rgb(255, 255, 255), // F: White
];

/// Convert EFI color index (0-15) to RGB color.
pub fn efi_to_rgb(index: u8) -> Color {
    EFI_PALETTE[(index & 0x0F) as usize]
}

/// Extract foreground color from EFI attribute.
pub fn attr_fg(attr: u8) -> Color {
    efi_to_rgb(attr & 0x0F)
}

/// Extract background color from EFI attribute.
pub fn attr_bg(attr: u8) -> Color {
    efi_to_rgb((attr >> 4) & 0x07)
}

/// Create EFI attribute from fg and bg indices.
pub const fn make_attr(fg: u8, bg: u8) -> u8 {
    (fg & 0x0F) | ((bg & 0x07) << 4)
}
