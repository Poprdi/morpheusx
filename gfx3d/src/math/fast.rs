/// Fast bit-level floating-point tricks.
///
/// These are the classic game-engine hacks: Quake III's fast inverse sqrt,
/// Carmack's fast reciprocal, IEEE-754 log2 approximation. Each trades a
/// tiny accuracy loss (~0.1-0.5%) for removing a hardware div/sqrt entirely.

/// Quake III fast inverse square root.
///
/// Two Newton-Raphson iterations give ~0.0003% max relative error.
/// One iteration gives ~0.175% which is fine for normals but not for lighting.
#[inline(always)]
pub fn inv_sqrt(x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }
    let half = 0.5 * x;
    let i = f32::to_bits(x);
    let i = 0x5f37_59df - (i >> 1); // magic constant (Chris Lomont's optimized)
    let y = f32::from_bits(i);
    let y = y * (1.5 - half * y * y); // 1st Newton iteration
    y * (1.5 - half * y * y) // 2nd Newton iteration
}

/// Fast reciprocal: 1/x using inverse sqrt trick.
///
/// Computes inv_sqrt(x) * inv_sqrt(x) * x ... nah, simpler: inv_sqrt(x*x) is
/// just 1/x when x > 0. But we lose precision for negative x.
/// Instead, use the IEEE-754 bit trick directly.
#[inline(always)]
pub fn fast_recip(x: f32) -> f32 {
    if x == 0.0 { return 0.0; }
    let i = f32::to_bits(x.abs());
    // Newton-Raphson: start with IEEE-754 trick for initial estimate
    let est = f32::from_bits(0x7ef0_0000 - i);
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    // Two refinement iterations: y = y * (2 - x*y)
    let est = est * (2.0 - x.abs() * est);
    let est = est * (2.0 - x.abs() * est);
    est * sign
}

/// Fast floor for positive floats.
#[inline(always)]
pub fn fast_floor(x: f32) -> i32 {
    let i = x as i32;
    if (i as f32) > x { i - 1 } else { i }
}

/// Fast approximate log2 using IEEE-754 exponent extraction.
///
/// Used for mipmap level selection: `level = log2(max(du/dx, dv/dy))`.
/// Error: ±0.08 (more than enough for LOD selection).
#[inline(always)]
pub fn fast_log2(x: f32) -> f32 {
    if x <= 0.0 { return -127.0; }
    let bits = f32::to_bits(x);
    let exp = ((bits >> 23) & 0xFF) as f32 - 127.0;
    let mantissa = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    let y = mantissa - 1.0;
    let log2_m = y * (1.3465557 + y * (-0.3606740 + y * 0.0141670));
    exp + log2_m
}

/// Fast 2^x using IEEE-754 bit reconstruction.
///
/// Used for fog density curves: `fog = 2^(-density * dist)`.
#[inline(always)]
pub fn fast_exp2(x: f32) -> f32 {
    if x < -126.0 { return 0.0; }
    if x > 128.0 { return f32::MAX; }
    let floor = fast_floor(x);
    let frac = x - floor as f32;
    // Polynomial approximation for 2^frac in [0, 1):
    let poly = 1.0 + frac * (0.6931 + frac * (0.2402 + frac * 0.0558));
    f32::from_bits(((floor + 127) as u32) << 23) * poly
}

/// Clamp float to [0, 1] without branching (uses min/max).
#[inline(always)]
pub fn saturate(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// Clamp i32 to u8 range.
#[inline(always)]
pub fn clamp_u8(x: i32) -> u8 {
    if x < 0 { 0 } else if x > 255 { 255 } else { x as u8 }
}

/// Integer lerp for color channels: a + (b - a) * t / 256, where t is [0, 255].
#[inline(always)]
pub fn ilerp_u8(a: u8, b: u8, t: u8) -> u8 {
    let a = a as u32;
    let b = b as u32;
    let t = t as u32;
    ((a * (255 - t) + b * t + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inv_sqrt_accuracy() {
        let vals = [1.0f32, 4.0, 9.0, 16.0, 100.0, 0.25, 0.01];
        for &v in &vals {
            let expected = 1.0 / (v as f64).sqrt();
            let got = inv_sqrt(v) as f64;
            let err = ((got - expected) / expected).abs();
            assert!(err < 0.001, "inv_sqrt({v}) = {got}, expected {expected}, err {err}");
        }
    }

    #[test]
    fn fast_recip_accuracy() {
        for &v in &[2.0f32, 3.0, 7.0, 100.0, -5.0, 0.01] {
            let expected = 1.0f64 / v as f64;
            let got = fast_recip(v) as f64;
            let err = ((got - expected) / expected).abs();
            assert!(err < 0.001, "fast_recip({v}) = {got}, expected {expected}, err {err}");
        }
    }

    #[test]
    fn fast_log2_accuracy() {
        for &v in &[1.0f32, 2.0, 4.0, 8.0, 0.5, 0.25, 3.7] {
            let expected = (v as f64).log2();
            let got = fast_log2(v) as f64;
            let err = (got - expected).abs();
            assert!(err < 0.15, "fast_log2({v}) = {got}, expected {expected}, err {err}");
        }
    }

    #[test]
    fn fast_exp2_accuracy() {
        for &v in &[0.0f32, 1.0, 2.0, 3.0, -1.0, -2.0, 0.5] {
            let expected = (v as f64).exp2();
            let got = fast_exp2(v) as f64;
            let err = ((got - expected) / expected).abs();
            assert!(err < 0.02, "fast_exp2({v}) = {got}, expected {expected}, err {err}");
        }
    }
}
