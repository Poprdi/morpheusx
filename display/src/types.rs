//! Core type definitions for the display crate.
//!
//! All types are `#[repr(C)]` for FFI compatibility with UEFI.

/// Pixel format in the framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PixelFormat {
    /// Red-Green-Blue-Reserved (RGBX), 8 bits each
    Rgbx = 0,
    /// Blue-Green-Red-Reserved (BGRX), 8 bits each - most common
    Bgrx = 1,
    /// Pixel defined by bitmask (not supported)
    BitMask = 2,
    /// No direct framebuffer access (not supported)
    BltOnly = 3,
}

impl Default for PixelFormat {
    fn default() -> Self {
        PixelFormat::Bgrx
    }
}

/// Information about the framebuffer obtained from GOP.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FramebufferInfo {
    /// Physical base address of the framebuffer.
    pub base: u64,
    /// Total size of the framebuffer in bytes.
    pub size: usize,
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in pixels.
    pub height: u32,
    /// Stride in bytes (may be > width * 4 due to padding).
    pub stride: u32,
    /// Pixel format.
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
    /// Check if framebuffer info is valid.
    pub fn is_valid(&self) -> bool {
        self.base != 0 && self.width > 0 && self.height > 0 && self.stride > 0
    }
}

/// 32-bit RGBA color.
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

    /// Create a color from RGB values.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Convert to a 32-bit value for BGRX format.
    pub const fn to_bgrx(&self) -> u32 {
        (self.b as u32) | ((self.g as u32) << 8) | ((self.r as u32) << 16)
    }

    /// Convert to a 32-bit value for RGBX format.
    pub const fn to_rgbx(&self) -> u32 {
        (self.r as u32) | ((self.g as u32) << 8) | ((self.b as u32) << 16)
    }
}

/// Text mode dimensions.
#[derive(Debug, Clone, Copy)]
pub struct TextMode {
    pub cols: usize,
    pub rows: usize,
}
