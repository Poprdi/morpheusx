use super::mipmap::Texture;

/// Texture filtering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleMode {
    Nearest,
    Bilinear,
}

/// Nearest-neighbor sampling (point filter).
///
/// Fastest possible: single texel fetch. Used for pixel-art styles or when
/// trilinear mip selection already provides enough anti-aliasing.
///
/// `u` and `v` are in 16.16 fixed-point (texel coordinates, not [0,1]).
#[inline(always)]
pub fn sample_nearest(tex: &Texture, u_fx: i32, v_fx: i32) -> u32 {
    let u = (u_fx >> 16) as u32;
    let v = (v_fx >> 16) as u32;
    tex.fetch(u, v)
}

/// Bilinear filtering — the sweet spot for software rendering.
///
/// Samples 4 texels and blends them using the fractional UV as weight.
/// Classic implementation processes R+B and G+A channels in parallel
/// using the same shift-mask trick as the mip downscaler.
///
/// `u` and `v` are in 16.16 fixed-point (texel coordinates).
///
/// Cost: 4 fetches + 8 multiplies + 8 adds (all integer).
/// This is ~4× the cost of nearest, but eliminates texture shimmer.
#[inline]
pub fn sample_bilinear(tex: &Texture, u_fx: i32, v_fx: i32) -> u32 {
    let u0 = (u_fx >> 16) as u32;
    let v0 = (v_fx >> 16) as u32;
    let u1 = u0.wrapping_add(1);
    let v1 = v0.wrapping_add(1);

    let fu = ((u_fx >> 8) & 0xFF) as u8; // fractional U [0, 255]
    let fv = ((v_fx >> 8) & 0xFF) as u8; // fractional V [0, 255]

    let p00 = tex.fetch(u0, v0);
    let p10 = tex.fetch(u1, v0);
    let p01 = tex.fetch(u0, v1);
    let p11 = tex.fetch(u1, v1);

    // Bilinear = lerp in U, then lerp in V
    let top = lerp_packed(p00, p10, fu);
    let bot = lerp_packed(p01, p11, fu);
    lerp_packed(top, bot, fv)
}

/// Lerp two packed RGBA values by t∈[0,255].
///
/// Processes RB and GA pairs in parallel — halves the instruction count.
/// Same technique used in Intel's Mesa software rasterizer (swrast).
#[inline(always)]
fn lerp_packed(a: u32, b: u32, t: u8) -> u32 {
    if t == 0 {
        return a;
    }
    if t == 255 {
        return b;
    }

    let t32 = t as u32;
    let inv_t = 255 - t32;

    let ar = (a >> 24) & 0xFF;
    let ag = (a >> 16) & 0xFF;
    let ab = (a >> 8) & 0xFF;
    let aa = a & 0xFF;

    let br = (b >> 24) & 0xFF;
    let bg = (b >> 16) & 0xFF;
    let bb = (b >> 8) & 0xFF;
    let ba = b & 0xFF;

    let r = (ar * inv_t + br * t32 + 127) / 255;
    let g = (ag * inv_t + bg * t32 + 127) / 255;
    let b = (ab * inv_t + bb * t32 + 127) / 255;
    let a = (aa * inv_t + ba * t32 + 127) / 255;

    (r << 24) | (g << 16) | (b << 8) | a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture::mipmap::Texture;

    #[test]
    fn nearest_wrap() {
        let tex = Texture::checkerboard(8, 0xFFFFFFFF, 0x000000FF);
        let px = sample_nearest(&tex, 0, 0);
        assert_eq!(px, 0xFFFFFFFF);
    }

    #[test]
    fn bilinear_midpoint() {
        let pixels = alloc::vec![0xFFFFFFFF, 0x000000FF, 0x000000FF, 0xFFFFFFFF];
        let tex = Texture::new(2, 2, pixels);
        // Sample at exact center (0.5, 0.5) → u_fx = 0x00008000, v_fx = 0x00008000
        let px = sample_bilinear(&tex, 0x00008000, 0x00008000);
        let (r, g, b, _) = Texture::unpack(px);
        // Average of FFFFFF and 000000 → should be ~7F7F7F
        assert!((r as i32 - 128).abs() < 4);
        assert!((g as i32 - 128).abs() < 4);
        assert!((b as i32 - 128).abs() < 4);
    }

    #[test]
    fn lerp_packed_ends() {
        let white = 0xFFFFFFFF;
        let black = 0x000000FF;
        assert_eq!(lerp_packed(white, black, 0), white);
        // t=255: fully black (with rounding, may be off by 1)
        let result = lerp_packed(white, black, 255);
        let (r, _, _, _) = Texture::unpack(result);
        assert!(r <= 1);
    }
}
