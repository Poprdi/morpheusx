/// Pre-computed sin/cos lookup table.
///
/// Instead of calling libm sin/cos (which we don't have in no_std anyway),
/// we build a 4096-entry table at init time using Bhaskara I's approximation
/// (7th-century formula, max error 0.0016 radians — tighter than most games need).
///
/// Table is indexed by angle × 4096 / (2π), so 4096 entries = full revolution.
/// This gives 0.088° angular resolution — far beyond what 1024×768 pixels can show.

use alloc::boxed::Box;
use super::fast;

const TABLE_SIZE: usize = 4096;
const TABLE_MASK: usize = TABLE_SIZE - 1;
const INV_TABLE: f32 = TABLE_SIZE as f32 / (2.0 * core::f32::consts::PI);
const TABLE_TO_RAD: f32 = (2.0 * core::f32::consts::PI) / TABLE_SIZE as f32;

pub struct TrigTable {
    sin_table: Box<[f32; TABLE_SIZE]>,
}

impl TrigTable {
    /// Build the table using Bhaskara I's rational approximation.
    ///
    /// For angle θ in [0, π]:
    ///   sin(θ) ≈ 16θ(π - θ) / (5π² - 4θ(π - θ))
    ///
    /// Max error: 0.00163 (0.16%). For comparison, a 256-entry linear-interp
    /// table (like Doom) has ~0.4% error.
    pub fn new() -> Self {
        let mut table = Box::new([0.0f32; TABLE_SIZE]);
        for i in 0..TABLE_SIZE {
            let rad = i as f32 * TABLE_TO_RAD;
            table[i] = bhaskara_sin(rad);
        }
        Self { sin_table: table }
    }

    #[inline(always)]
    pub fn sin(&self, radians: f32) -> f32 {
        let (sin, _) = self.sin_cos(radians);
        sin
    }

    #[inline(always)]
    pub fn cos(&self, radians: f32) -> f32 {
        let (_, cos) = self.sin_cos(radians);
        cos
    }

    /// sin and cos in one call (avoids redundant index math).
    #[inline]
    pub fn sin_cos(&self, radians: f32) -> (f32, f32) {
        let two_pi = 2.0 * core::f32::consts::PI;
        let mut angle = radians % two_pi;
        if angle < 0.0 {
            angle += two_pi;
        }
        let turn = angle * INV_TABLE;
        let base = turn as usize;
        let frac = turn - base as f32;

        let si0 = base & TABLE_MASK;
        let si1 = (si0 + 1) & TABLE_MASK;
        let ci0 = (si0 + (TABLE_SIZE >> 2)) & TABLE_MASK;
        let ci1 = (ci0 + 1) & TABLE_MASK;

        let sin0 = self.sin_table[si0];
        let sin1 = self.sin_table[si1];
        let cos0 = self.sin_table[ci0];
        let cos1 = self.sin_table[ci1];

        let mut sin = sin0 + (sin1 - sin0) * frac;
        let mut cos = cos0 + (cos1 - cos0) * frac;

        let len_sq = sin * sin + cos * cos;
        if len_sq > 1e-12 {
            let inv_len = fast::inv_sqrt(len_sq);
            sin *= inv_len;
            cos *= inv_len;
        }

        (sin, cos)
    }

    /// Atan2 approximation (useful for angle-based effects, not in hot render path).
    /// Max error ~0.28° — uses the 7th-order polynomial from NVIDIA's GPU gems.
    pub fn atan2(y: f32, x: f32) -> f32 {
        if x == 0.0 && y == 0.0 { return 0.0; }
        let ax = if x < 0.0 { -x } else { x };
        let ay = if y < 0.0 { -y } else { y };
        let (mn, mx) = if ax < ay { (ax, ay) } else { (ay, ax) };
        let a = mn / mx;
        // Polynomial: max error 0.0049 rad = 0.28°
        let s = a * a;
        let r = ((-0.0464964749 * s + 0.15931422) * s - 0.327622764) * s * a + a;
        let r = if ay > ax { 1.5707963 - r } else { r };
        let r = if x < 0.0 { core::f32::consts::PI - r } else { r };
        if y < 0.0 { -r } else { r }
    }
}

/// Bhaskara I's sine approximation (7th century CE, India).
///
/// For θ in [0, π]: sin(θ) ≈ 16θ(π-θ) / (5π² - 4θ(π-θ))
/// We extend to full [0, 2π] via symmetry.
fn bhaskara_sin(theta: f32) -> f32 {
    let pi = core::f32::consts::PI;
    let two_pi = 2.0 * pi;

    // Normalize to [0, 2π)
    let mut t = theta % two_pi;
    if t < 0.0 { t += two_pi; }

    let (t_local, sign) = if t > pi {
        (t - pi, -1.0f32)
    } else {
        (t, 1.0f32)
    };

    let complement = pi - t_local;
    let product = t_local * complement;
    let numerator = 16.0 * product;
    let denominator = 5.0 * pi * pi - 4.0 * product;

    if denominator.abs() < 1e-10 {
        return 0.0;
    }

    sign * numerator / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bhaskara_accuracy() {
        let table = TrigTable::new();
        let pi = core::f32::consts::PI;
        let half_pi = core::f32::consts::FRAC_PI_2;
        let two_pi = 2.0 * pi;

        assert!(table.sin(0.0).abs() < 0.002);
        assert!((table.sin(half_pi) - 1.0).abs() < 0.01);
        assert!(table.sin(pi).abs() < 0.01);
        assert!((table.sin(3.0 * half_pi) + 1.0).abs() < 0.01);
        assert!(table.sin(two_pi).abs() < 0.01);
    }

    #[test]
    fn sin_cos_is_unit_circle() {
        let table = TrigTable::new();
        for i in 0..1024 {
            let a = (i as f32) * 0.01;
            let (s, c) = table.sin_cos(a);
            let n = s * s + c * c;
            assert!((n - 1.0).abs() < 0.002, "angle={a} s2+c2={n}");
        }
    }

}
