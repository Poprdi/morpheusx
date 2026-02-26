/// 16.16 fixed-point for sub-pixel precision in rasterizer inner loops.
///
/// Used where f32→int conversion overhead dominates (edge stepping, UV interpolation).
/// The rasterizer pre-converts to Fx16 before the scanline loop, then steps with
/// pure integer adds — no FPU traffic in the hot path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Fx16(pub i32);

const SHIFT: u32 = 16;
const ONE: i32 = 1 << SHIFT;
const HALF: i32 = ONE >> 1;
const FRAC_MASK: i32 = ONE - 1;

impl Fx16 {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(ONE);
    pub const HALF: Self = Self(HALF);
    pub const NEG_ONE: Self = Self(-ONE);
    pub const EPSILON: Self = Self(1);

    #[inline(always)]
    pub const fn from_i32(v: i32) -> Self {
        Self(v << SHIFT)
    }

    #[inline(always)]
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    #[inline(always)]
    pub fn from_f32(v: f32) -> Self {
        Self((v * ONE as f32) as i32)
    }

    #[inline(always)]
    pub const fn to_i32(self) -> i32 {
        self.0 >> SHIFT
    }

    #[inline(always)]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / ONE as f32
    }

    #[inline(always)]
    pub const fn frac(self) -> i32 {
        self.0 & FRAC_MASK
    }

    /// Round to nearest integer.
    #[inline(always)]
    pub const fn round(self) -> i32 {
        (self.0 + HALF) >> SHIFT
    }

    /// Floor: bias toward negative infinity.
    #[inline(always)]
    pub const fn floor(self) -> i32 {
        self.0 >> SHIFT
    }

    /// Ceil: bias toward positive infinity.
    #[inline(always)]
    pub const fn ceil(self) -> i32 {
        (self.0 + FRAC_MASK) >> SHIFT
    }

    /// Fixed × fixed with 64-bit intermediate (no overflow on 16.16 products).
    #[inline(always)]
    pub const fn mul(self, rhs: Self) -> Self {
        Self(((self.0 as i64 * rhs.0 as i64) >> SHIFT) as i32)
    }

    /// Fixed / fixed with 64-bit pre-shift (full precision, no truncation).
    #[inline(always)]
    pub const fn div(self, rhs: Self) -> Self {
        if rhs.0 == 0 {
            return Self(if self.0 >= 0 { i32::MAX } else { i32::MIN });
        }
        Self((((self.0 as i64) << SHIFT) / rhs.0 as i64) as i32)
    }

    #[inline(always)]
    pub const fn abs(self) -> Self {
        if self.0 < 0 { Self(-self.0) } else { self }
    }

    /// Linear interpolation: self + (other - self) * t, where t is Fx16 in [0, 1].
    #[inline(always)]
    pub const fn lerp(self, other: Self, t: Self) -> Self {
        let diff = other.0 - self.0;
        Self(self.0 + ((diff as i64 * t.0 as i64) >> SHIFT) as i32)
    }
}

impl core::ops::Add for Fx16 {
    type Output = Self;
    #[inline(always)]
    fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0) }
}

impl core::ops::Sub for Fx16 {
    type Output = Self;
    #[inline(always)]
    fn sub(self, rhs: Self) -> Self { Self(self.0 - rhs.0) }
}

impl core::ops::Neg for Fx16 {
    type Output = Self;
    #[inline(always)]
    fn neg(self) -> Self { Self(-self.0) }
}

impl core::ops::AddAssign for Fx16 {
    #[inline(always)]
    fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0; }
}

impl core::ops::SubAssign for Fx16 {
    #[inline(always)]
    fn sub_assign(&mut self, rhs: Self) { self.0 -= rhs.0; }
}

impl core::ops::Shr<u32> for Fx16 {
    type Output = Self;
    #[inline(always)]
    fn shr(self, rhs: u32) -> Self { Self(self.0 >> rhs) }
}

impl core::ops::Shl<u32> for Fx16 {
    type Output = Self;
    #[inline(always)]
    fn shl(self, rhs: u32) -> Self { Self(self.0 << rhs) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert_eq!(Fx16::from_i32(42).to_i32(), 42);
        assert_eq!(Fx16::from_i32(-7).to_i32(), -7);
    }

    #[test]
    fn mul_precision() {
        let a = Fx16::from_f32(3.5);
        let b = Fx16::from_f32(2.0);
        let c = a.mul(b);
        assert_eq!(c.to_i32(), 7);
    }

    #[test]
    fn lerp_midpoint() {
        let a = Fx16::from_i32(0);
        let b = Fx16::from_i32(100);
        let mid = a.lerp(b, Fx16::HALF);
        assert_eq!(mid.to_i32(), 50);
    }
}
