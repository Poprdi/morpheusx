#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PixelFormat {
    Bgrx = 0,
    Rgbx = 1,
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
    pub const GREEN: Color = Color::rgb(0, 170, 0);
    pub const BLUE: Color = Color::rgb(0, 0, 255);
    pub const YELLOW: Color = Color::rgb(255, 255, 85);
    pub const CYAN: Color = Color::rgb(0, 170, 170);
    pub const MAGENTA: Color = Color::rgb(170, 0, 170);
    pub const LIGHT_GRAY: Color = Color::rgb(170, 170, 170);
    pub const DARK_GRAY: Color = Color::rgb(85, 85, 85);
    pub const LIGHT_GREEN: Color = Color::rgb(85, 255, 85);
    pub const DARK_GREEN: Color = Color::rgb(0, 85, 0);
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[inline]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[inline]
    pub const fn to_packed(self, format: PixelFormat) -> u32 {
        match format {
            PixelFormat::Bgrx => {
                (self.b as u32)
                    | ((self.g as u32) << 8)
                    | ((self.r as u32) << 16)
                    | ((self.a as u32) << 24)
            }
            PixelFormat::Rgbx => {
                (self.r as u32)
                    | ((self.g as u32) << 8)
                    | ((self.b as u32) << 16)
                    | ((self.a as u32) << 24)
            }
        }
    }

    #[inline]
    pub const fn from_packed(packed: u32, format: PixelFormat) -> Self {
        match format {
            PixelFormat::Bgrx => Self {
                b: packed as u8,
                g: (packed >> 8) as u8,
                r: (packed >> 16) as u8,
                a: (packed >> 24) as u8,
            },
            PixelFormat::Rgbx => Self {
                r: packed as u8,
                g: (packed >> 8) as u8,
                b: (packed >> 16) as u8,
                a: (packed >> 24) as u8,
            },
        }
    }

    #[inline]
    #[must_use]
    pub fn blend_over(self, dst: Color) -> Color {
        if self.a == 255 {
            return self;
        }
        if self.a == 0 {
            return dst;
        }
        let sa = self.a as u32;
        let inv_sa = 255 - sa;
        Color {
            r: ((self.r as u32 * sa + dst.r as u32 * inv_sa + 128) >> 8) as u8,
            g: ((self.g as u32 * sa + dst.g as u32 * inv_sa + 128) >> 8) as u8,
            b: ((self.b as u32 * sa + dst.b as u32 * inv_sa + 128) >> 8) as u8,
            a: ((sa + (dst.a as u32 * inv_sa + 128) >> 8).min(255)) as u8,
        }
    }

    #[inline]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self {
            r: self.r,
            g: self.g,
            b: self.b,
            a,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_sets_full_alpha() {
        let c = Color::rgb(10, 20, 30);
        assert_eq!(c.a, 255);
    }

    #[test]
    fn bgrx_roundtrip() {
        let c = Color::rgba(0xAA, 0xBB, 0xCC, 0xFF);
        let packed = c.to_packed(PixelFormat::Bgrx);
        let back = Color::from_packed(packed, PixelFormat::Bgrx);
        assert_eq!(c, back);
    }

    #[test]
    fn rgbx_roundtrip() {
        let c = Color::rgba(0x11, 0x22, 0x33, 0xDD);
        let packed = c.to_packed(PixelFormat::Rgbx);
        let back = Color::from_packed(packed, PixelFormat::Rgbx);
        assert_eq!(c, back);
    }

    #[test]
    fn blend_opaque_returns_src() {
        let src = Color::rgb(255, 0, 0);
        let dst = Color::rgb(0, 0, 255);
        assert_eq!(src.blend_over(dst), src);
    }

    #[test]
    fn blend_transparent_returns_dst() {
        let src = Color::rgba(255, 0, 0, 0);
        let dst = Color::rgb(0, 0, 255);
        assert_eq!(src.blend_over(dst), dst);
    }

    #[test]
    fn blend_half_alpha() {
        let src = Color::rgba(255, 0, 0, 128);
        let dst = Color::rgb(0, 0, 255);
        let result = src.blend_over(dst);
        assert!(result.r > 100 && result.r < 160);
        assert!(result.b > 100 && result.b < 160);
    }
}
