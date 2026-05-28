//! Display types. `#[repr(C)]` for UEFI FFI compatibility.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[derive(Default)]
pub enum PixelFormat {
    Rgbx = 0,
    #[default]
    Bgrx = 1,
    /// BitMask and BltOnly are reported by GOP but not supported here.
    BitMask = 2,
    BltOnly = 3,
}

/// Framebuffer info from GOP. `stride` is bytes and may exceed width*4.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FramebufferInfo {
    pub base: u64,
    pub size: usize,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
}

impl Default for FramebufferInfo {
    fn default() -> Self {
        Self {
            base: 0,
            size: 0,
            width: 0,
            height: 0,
            stride: 0,
            format: PixelFormat::Bgrx,
        }
    }
}

impl FramebufferInfo {
    pub fn is_valid(&self) -> bool {
        self.base != 0 && self.width > 0 && self.height > 0 && self.stride > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const RED: Color = Color::rgb(255, 0, 0);
    pub const GREEN: Color = Color::rgb(0, 255, 0);
    pub const BLUE: Color = Color::rgb(0, 0, 255);
    pub const YELLOW: Color = Color::rgb(255, 255, 0);
    pub const CYAN: Color = Color::rgb(0, 255, 255);
    pub const MAGENTA: Color = Color::rgb(255, 0, 255);
    pub const LIGHT_GRAY: Color = Color::rgb(170, 170, 170);
    pub const DARK_GRAY: Color = Color::rgb(85, 85, 85);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn to_bgrx(self) -> u32 {
        (self.b as u32) | ((self.g as u32) << 8) | ((self.r as u32) << 16)
    }

    pub const fn to_rgbx(self) -> u32 {
        (self.r as u32) | ((self.g as u32) << 8) | ((self.b as u32) << 16)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TextMode {
    pub cols: usize,
    pub rows: usize,
}
