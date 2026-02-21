pub mod vga8x16;

pub use vga8x16::FONT_DATA;

pub const FONT_WIDTH: u32 = 8;
pub const FONT_HEIGHT: u32 = 16;

pub fn get_glyph(c: char) -> Option<&'static [u8; 16]> {
    let idx = c as usize;
    if idx >= 0x20 && idx <= 0x7E {
        Some(&FONT_DATA[idx - 0x20])
    } else {
        None
    }
}

pub fn get_glyph_or_space(c: char) -> &'static [u8; 16] {
    get_glyph(c).unwrap_or(&FONT_DATA[0])
}
