// IEEE-754 bit tricks. Trade ~0.1-0.5% accuracy for no hardware div/sqrt.

/// Quake III rsqrt with 2 Newton iterations (~0.0003% rel error).
#[inline(always)]
pub fn inv_sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let half = 0.5 * x;
    let i = f32::to_bits(x);
    let i = 0x5f37_59df - (i >> 1); // Lomont's refined constant
    let y = f32::from_bits(i);
    let y = y * (1.5 - half * y * y);
    y * (1.5 - half * y * y)
}

/// 1/x via IEEE-754 seed + 2 Newton iterations.
#[inline(always)]
pub fn fast_recip(x: f32) -> f32 {
    if x == 0.0 {
        return 0.0;
    }
    let i = f32::to_bits(x.abs());
    let est = f32::from_bits(0x7ef0_0000 - i);
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let est = est * (2.0 - x.abs() * est);
    let est = est * (2.0 - x.abs() * est);
    est * sign
}

#[inline(always)]
pub fn fast_floor(x: f32) -> i32 {
    let i = x as i32;
    if (i as f32) > x {
        i - 1
    } else {
        i
    }
}

/// IEEE-754 log2 approximation. Error ±0.08 — sufficient for mipmap LOD.
#[inline(always)]
pub fn fast_log2(x: f32) -> f32 {
    if x <= 0.0 {
        return -127.0;
    }
    let bits = f32::to_bits(x);
    let exp = ((bits >> 23) & 0xFF) as f32 - 127.0;
    let mantissa = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    let y = mantissa - 1.0;
    let log2_m = y * (1.3465557 + y * (-0.3606740 + y * 0.0141670));
    exp + log2_m
}

/// IEEE-754 2^x reconstruction; used for exponential fog.
#[inline(always)]
pub fn fast_exp2(x: f32) -> f32 {
    if x < -126.0 {
        return 0.0;
    }
    if x > 128.0 {
        return f32::MAX;
    }
    let floor = fast_floor(x);
    let frac = x - floor as f32;
    let poly = 1.0 + frac * (core::f32::consts::LN_2 + frac * (0.2402 + frac * 0.0558));
    f32::from_bits(((floor + 127) as u32) << 23) * poly
}

#[inline(always)]
pub fn saturate(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 {
        1.0
    } else {
        x
    }
}

#[inline(always)]
pub fn clamp_u8(x: i32) -> u8 {
    if x < 0 {
        0
    } else if x > 255 {
        255
    } else {
        x as u8
    }
}

/// t in [0,255].
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
            assert!(
                err < 0.001,
                "inv_sqrt({v}) = {got}, expected {expected}, err {err}"
            );
        }
    }

    #[test]
    fn fast_recip_accuracy() {
        for &v in &[2.0f32, 3.0, 7.0, 100.0, -5.0, 0.01] {
            let expected = 1.0f64 / v as f64;
            let got = fast_recip(v) as f64;
            let err = ((got - expected) / expected).abs();
            assert!(
                err < 0.001,
                "fast_recip({v}) = {got}, expected {expected}, err {err}"
            );
        }
    }

    #[test]
    fn fast_log2_accuracy() {
        for &v in &[1.0f32, 2.0, 4.0, 8.0, 0.5, 0.25, 3.7] {
            let expected = (v as f64).log2();
            let got = fast_log2(v) as f64;
            let err = (got - expected).abs();
            assert!(
                err < 0.15,
                "fast_log2({v}) = {got}, expected {expected}, err {err}"
            );
        }
    }

    #[test]
    fn fast_exp2_accuracy() {
        for &v in &[0.0f32, 1.0, 2.0, 3.0, -1.0, -2.0, 0.5] {
            let expected = (v as f64).exp2();
            let got = fast_exp2(v) as f64;
            let err = ((got - expected) / expected).abs();
            assert!(
                err < 0.02,
                "fast_exp2({v}) = {got}, expected {expected}, err {err}"
            );
        }
    }
}
