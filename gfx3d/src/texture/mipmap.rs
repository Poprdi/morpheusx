use alloc::vec::Vec;

/// RGBA8888 (R in MSB), power-of-two dims. Format-independent of the framebuffer.
#[derive(Clone)]
pub struct Texture {
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub width_mask: u32,
    pub height_mask: u32,
    pub width_shift: u32, // log2(width)
}

impl Texture {
    pub fn new(width: u32, height: u32, pixels: Vec<u32>) -> Self {
        debug_assert!(width.is_power_of_two() && height.is_power_of_two());
        Self {
            pixels,
            width,
            height,
            width_mask: width - 1,
            height_mask: height - 1,
            width_shift: width.trailing_zeros(),
        }
    }

    pub fn solid(rgba: u32) -> Self {
        Self::new(1, 1, alloc::vec![rgba])
    }

    pub fn checkerboard(size: u32, c0: u32, c1: u32) -> Self {
        let mut pixels = Vec::with_capacity((size * size) as usize);
        for y in 0..size {
            for x in 0..size {
                let checker = ((x >> 3) ^ (y >> 3)) & 1;
                pixels.push(if checker == 0 { c0 } else { c1 });
            }
        }
        Self::new(size, size, pixels)
    }

    #[inline(always)]
    pub fn fetch(&self, u: u32, v: u32) -> u32 {
        let u = u & self.width_mask;
        let v = v & self.height_mask;
        let idx = (v << self.width_shift) | u;
        if let Some(&px) = self.pixels.get(idx as usize) {
            px
        } else {
            0
        }
    }

    #[inline(always)]
    pub fn unpack(packed: u32) -> (u8, u8, u8, u8) {
        let r = (packed >> 24) as u8;
        let g = (packed >> 16) as u8;
        let b = (packed >> 8) as u8;
        let a = packed as u8;
        (r, g, b, a)
    }

    #[inline(always)]
    pub fn pack(r: u8, g: u8, b: u8, a: u8) -> u32 {
        (r as u32) << 24 | (g as u32) << 16 | (b as u32) << 8 | a as u32
    }
}

/// Box-filter mip chain. Level 0 is base; selected via fast_log2(texel/pixel).
pub struct MipChain {
    pub levels: Vec<Texture>,
}

impl MipChain {
    pub fn build(base: Texture) -> Self {
        let mut levels = Vec::new();
        let mut current = base;

        loop {
            let w = current.width;
            let h = current.height;
            levels.push(current);

            if w == 1 && h == 1 {
                break;
            }

            let nw = (w >> 1).max(1);
            let nh = (h >> 1).max(1);
            let parent = levels.last().unwrap();
            let mut pixels = Vec::with_capacity((nw * nh) as usize);

            for y in 0..nh {
                for x in 0..nw {
                    let sx = x << 1;
                    let sy = y << 1;
                    let p00 = parent.fetch(sx, sy);
                    let p10 = parent.fetch(sx + 1, sy);
                    let p01 = parent.fetch(sx, sy + 1);
                    let p11 = parent.fetch(sx + 1, sy + 1);
                    pixels.push(avg4(p00, p10, p01, p11));
                }
            }

            current = Texture::new(nw, nh, pixels);
        }

        Self { levels }
    }

    /// level = log2(texels_per_pixel), clamped.
    #[inline]
    pub fn select_level(&self, texels_per_pixel: f32) -> usize {
        if texels_per_pixel <= 1.0 {
            return 0;
        }
        let level = crate::math::fast::fast_log2(texels_per_pixel) as usize;
        level.min(self.levels.len() - 1)
    }

    pub fn level(&self, idx: usize) -> &Texture {
        &self.levels[idx.min(self.levels.len() - 1)]
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }
}

/// 4-pixel box average. Interleaved RB/GA channels via shift-mask (Quake 2 trick).
#[inline]
fn avg4(a: u32, b: u32, c: u32, d: u32) -> u32 {
    const RB_MASK: u32 = 0x00FF00FF;

    let rb = ((a & RB_MASK) + (b & RB_MASK) + (c & RB_MASK) + (d & RB_MASK) + 0x00020002) >> 2;
    let ga = (((a >> 8) & RB_MASK)
        + ((b >> 8) & RB_MASK)
        + ((c >> 8) & RB_MASK)
        + ((d >> 8) & RB_MASK)
        + 0x00020002)
        >> 2;

    (rb & RB_MASK) | ((ga & RB_MASK) << 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_texture() {
        let t = Texture::solid(0xFF0000FF);
        assert_eq!(t.fetch(0, 0), 0xFF0000FF);
        assert_eq!(t.width, 1);
    }

    #[test]
    fn mip_chain_sizes() {
        let base = Texture::checkerboard(64, 0xFFFFFFFF, 0x000000FF);
        let chain = MipChain::build(base);
        assert_eq!(chain.levels[0].width, 64);
        assert_eq!(chain.levels[1].width, 32);
        assert_eq!(chain.levels[2].width, 16);
        assert_eq!(chain.levels.last().unwrap().width, 1);
    }

    #[test]
    fn avg4_correctness() {
        let white = 0xFFFFFFFF;
        let black = 0x000000FF;
        let avg = avg4(white, white, black, black);
        let (r, g, b, _a) = Texture::unpack(avg);
        assert!((r as i32 - 128).abs() <= 1);
        assert!((g as i32 - 128).abs() <= 1);
        assert!((b as i32 - 128).abs() <= 1);
    }
}
