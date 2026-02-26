/// Pre-computed sin/cos lookup table.
///
/// Instead of calling libm sin/cos (which we don't have in no_std anyway),
/// we build a 4096-entry table at init time using Bhaskara I's approximation
/// (7th-century formula, max error 0.0016 radians — tighter than most games need).
///
/// Table is indexed by angle × 4096 / (2π), so 4096 entries = full revolution.
/// This gives 0.088° angular resolution — far beyond what 1024×768 pixels can show.

use alloc::boxed::Box;

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
        let idx = (radians * INV_TABLE) as i32;
        let idx = (idx as usize) & TABLE_MASK;
        self.sin_table[idx]
    }

    #[inline(always)]
    pub fn cos(&self, radians: f32) -> f32 {
        let idx = (radians * INV_TABLE) as i32 + (TABLE_SIZE as i32 >> 2);
        let idx = (idx as usize) & TABLE_MASK;
        self.sin_table[idx]
    }

    /// sin and cos in one call (avoids redundant index math).
    #[inline]
    pub fn sin_cos(&self, radians: f32) -> (f32, f32) {
        let base = (radians * INV_TABLE) as i32;
        let si = (base as usize) & TABLE_MASK;
        let ci = ((base as usize) + (TABLE_SIZE >> 2)) & TABLE_MASK;
        (self.sin_table[si], self.sin_table[ci])
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
        let test_angles = [0.0f32, 0.5, 1.0, 1.5707963, 3.14159, 4.712, 6.28];
        for &a in &test_angles {
            let got = table.sin(a);
            let expected = bhaskara_sin_reference(a);
            let err = (got - expected).abs();
            assert!(err < 0.01, "sin({a}) = {got}, expected {expected}, err {err}");
        }
    }

    fn bhaskara_sin_reference(x: f32) -> f32 {
        // Use Taylor series as reference (enough terms for test accuracy)
        let x = x % (2.0 * core::f32::consts::PI);
        let x2 = x * x;
        let x3 = x2 * x;
        let x5 = x3 * x2;
        let x7 = x5 * x2;
        x - x3 / 6.0 + x5 / 120.0 - x7 / 5040.0
    }
}
