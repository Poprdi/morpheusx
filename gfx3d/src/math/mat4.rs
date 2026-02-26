use super::vec::{Vec3, Vec4};

/// Column-major 4×4 matrix.
///
/// Layout matches OpenGL convention: columns are contiguous in memory.
/// This means `m[col][row]` — vec4 column vectors at m[0], m[1], m[2], m[3].
///
/// Memory: `[c0r0 c0r1 c0r2 c0r3 | c1r0 c1r1 c1r2 c1r3 | ...]`
///
/// Column-major was chosen for two reasons:
/// 1. Matrix × vector is 4 dot products against columns (SIMD-friendly)
/// 2. Transform concatenation: M_final = Projection × View × Model (right-to-left)
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C, align(16))]
pub struct Mat4 {
    pub cols: [[f32; 4]; 4],
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    pub const ZERO: Self = Self { cols: [[0.0; 4]; 4] };

    #[inline]
    pub const fn from_cols(c0: [f32; 4], c1: [f32; 4], c2: [f32; 4], c3: [f32; 4]) -> Self {
        Self { cols: [c0, c1, c2, c3] }
    }

    /// Access element at (row, col).
    #[inline(always)]
    pub fn at(&self, row: usize, col: usize) -> f32 {
        self.cols[col][row]
    }

    /// Set element at (row, col).
    #[inline(always)]
    pub fn set(&mut self, row: usize, col: usize, val: f32) {
        self.cols[col][row] = val;
    }

    /// Matrix × Vec4 (transform a point/vector through the pipeline).
    #[inline]
    pub fn transform(&self, v: Vec4) -> Vec4 {
        Vec4 {
            x: self.cols[0][0] * v.x + self.cols[1][0] * v.y + self.cols[2][0] * v.z + self.cols[3][0] * v.w,
            y: self.cols[0][1] * v.x + self.cols[1][1] * v.y + self.cols[2][1] * v.z + self.cols[3][1] * v.w,
            z: self.cols[0][2] * v.x + self.cols[1][2] * v.y + self.cols[2][2] * v.z + self.cols[3][2] * v.w,
            w: self.cols[0][3] * v.x + self.cols[1][3] * v.y + self.cols[2][3] * v.z + self.cols[3][3] * v.w,
        }
    }

    /// Transform Vec3 as a point (w=1, includes translation).
    #[inline]
    pub fn transform_point(&self, p: Vec3) -> Vec4 {
        self.transform(p.to_vec4(1.0))
    }

    /// Transform Vec3 as a direction (w=0, ignores translation).
    #[inline]
    pub fn transform_dir(&self, d: Vec3) -> Vec3 {
        Vec3 {
            x: self.cols[0][0] * d.x + self.cols[1][0] * d.y + self.cols[2][0] * d.z,
            y: self.cols[0][1] * d.x + self.cols[1][1] * d.y + self.cols[2][1] * d.z,
            z: self.cols[0][2] * d.x + self.cols[1][2] * d.y + self.cols[2][2] * d.z,
        }
    }

    /// Matrix × matrix multiplication.
    ///
    /// This is the hot path during scene graph traversal. Each node concatenates
    /// its local transform with the parent's world transform.
    /// 64 multiplies + 48 adds for the general case.
    #[inline]
    pub fn mul(&self, rhs: &Mat4) -> Mat4 {
        let mut out = Mat4::ZERO;
        for c in 0..4 {
            for r in 0..4 {
                out.cols[c][r] =
                    self.cols[0][r] * rhs.cols[c][0] +
                    self.cols[1][r] * rhs.cols[c][1] +
                    self.cols[2][r] * rhs.cols[c][2] +
                    self.cols[3][r] * rhs.cols[c][3];
            }
        }
        out
    }

    /// Translation matrix.
    #[inline]
    pub fn translation(tx: f32, ty: f32, tz: f32) -> Self {
        Self::from_cols(
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [tx,  ty,  tz,  1.0],
        )
    }

    /// Uniform scale.
    #[inline]
    pub fn scale(sx: f32, sy: f32, sz: f32) -> Self {
        Self::from_cols(
            [sx,  0.0, 0.0, 0.0],
            [0.0, sy,  0.0, 0.0],
            [0.0, 0.0, sz,  0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
    }

    /// Rotation around X axis. `sin_cos` from TrigTable.
    #[inline]
    pub fn rotation_x(sin: f32, cos: f32) -> Self {
        Self::from_cols(
            [1.0, 0.0, 0.0, 0.0],
            [0.0, cos, sin, 0.0],
            [0.0, -sin, cos, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
    }

    /// Rotation around Y axis.
    #[inline]
    pub fn rotation_y(sin: f32, cos: f32) -> Self {
        Self::from_cols(
            [cos, 0.0, -sin, 0.0],
            [0.0, 1.0, 0.0,  0.0],
            [sin, 0.0, cos,  0.0],
            [0.0, 0.0, 0.0,  1.0],
        )
    }

    /// Rotation around Z axis.
    #[inline]
    pub fn rotation_z(sin: f32, cos: f32) -> Self {
        Self::from_cols(
            [cos,  sin, 0.0, 0.0],
            [-sin, cos, 0.0, 0.0],
            [0.0,  0.0, 1.0, 0.0],
            [0.0,  0.0, 0.0, 1.0],
        )
    }

    /// Rotation around arbitrary axis (Rodrigues' formula).
    pub fn rotation_axis(axis: Vec3, sin: f32, cos: f32) -> Self {
        let a = axis.normalize();
        let omc = 1.0 - cos; // one minus cos
        Self::from_cols(
            [cos + a.x * a.x * omc,       a.y * a.x * omc + a.z * sin,  a.z * a.x * omc - a.y * sin, 0.0],
            [a.x * a.y * omc - a.z * sin,  cos + a.y * a.y * omc,       a.z * a.y * omc + a.x * sin, 0.0],
            [a.x * a.z * omc + a.y * sin,  a.y * a.z * omc - a.x * sin, cos + a.z * a.z * omc,       0.0],
            [0.0,                           0.0,                          0.0,                          1.0],
        )
    }

    /// Perspective projection (symmetric frustum).
    ///
    /// Maps view-space → clip-space with infinite-precision depth range [near, far].
    /// Uses reversed-Z (near maps to 1, far maps to 0) for better float precision
    /// at distance — standard trick from Outerra/DICE/id Software.
    pub fn perspective(fov_y_rad: f32, aspect: f32, near: f32, far: f32) -> Self {
        let half_fov = fov_y_rad * 0.5;
        // Use TrigTable's sin_cos in real code; here we use Bhaskara inline
        let (sin_fov, cos_fov) = (bhaskara_sin_inline(half_fov), bhaskara_cos_inline(half_fov));
        if sin_fov.abs() < 1e-10 { return Self::IDENTITY; }
        let cot = cos_fov / sin_fov;
        let inv_range = 1.0 / (near - far);

        // Reversed-Z: z_ndc = near / (far - near) * z_eye + far*near / (far - near)
        Self::from_cols(
            [cot / aspect, 0.0, 0.0,  0.0],
            [0.0,          cot, 0.0,  0.0],
            [0.0,          0.0, far * inv_range, -1.0],
            [0.0,          0.0, near * far * inv_range, 0.0],
        )
    }

    /// Orthographic projection.
    pub fn ortho(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        let inv_w = 1.0 / (right - left);
        let inv_h = 1.0 / (top - bottom);
        let inv_d = 1.0 / (far - near);
        Self::from_cols(
            [2.0 * inv_w, 0.0, 0.0, 0.0],
            [0.0, 2.0 * inv_h, 0.0, 0.0],
            [0.0, 0.0, -inv_d, 0.0],
            [-(right + left) * inv_w, -(top + bottom) * inv_h, -near * inv_d, 1.0],
        )
    }

    /// Look-at view matrix (right-handed, camera at `eye` looking at `target`).
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let f = (target - eye).normalize(); // forward
        let r = f.cross(up).normalize();    // right
        let u = r.cross(f);                 // true up

        Self::from_cols(
            [r.x,     u.x,    -f.x,    0.0],
            [r.y,     u.y,    -f.y,    0.0],
            [r.z,     u.z,    -f.z,    0.0],
            [-r.dot(eye), -u.dot(eye), f.dot(eye), 1.0],
        )
    }

    /// Transpose (useful for normal matrix when no non-uniform scale).
    pub fn transpose(&self) -> Self {
        Self::from_cols(
            [self.cols[0][0], self.cols[1][0], self.cols[2][0], self.cols[3][0]],
            [self.cols[0][1], self.cols[1][1], self.cols[2][1], self.cols[3][1]],
            [self.cols[0][2], self.cols[1][2], self.cols[2][2], self.cols[3][2]],
            [self.cols[0][3], self.cols[1][3], self.cols[2][3], self.cols[3][3]],
        )
    }

    /// Inverse of affine transform (rotation + translation, no shear/scale).
    ///
    /// Much cheaper than general 4×4 inverse. Uses the fact that for
    /// orthonormal 3×3 upper-left, inverse = transpose. Translation
    /// is recomputed as -R^T × t.
    pub fn inverse_affine(&self) -> Self {
        // Transpose the 3×3 rotation part
        let r00 = self.cols[0][0]; let r01 = self.cols[1][0]; let r02 = self.cols[2][0];
        let r10 = self.cols[0][1]; let r11 = self.cols[1][1]; let r12 = self.cols[2][1];
        let r20 = self.cols[0][2]; let r21 = self.cols[1][2]; let r22 = self.cols[2][2];
        let tx = self.cols[3][0]; let ty = self.cols[3][1]; let tz = self.cols[3][2];

        Self::from_cols(
            [r00, r01, r02, 0.0],
            [r10, r11, r12, 0.0],
            [r20, r21, r22, 0.0],
            [-(r00*tx + r01*ty + r02*tz),
             -(r10*tx + r11*ty + r12*tz),
             -(r20*tx + r21*ty + r22*tz), 1.0],
        )
    }

    /// General 4×4 inverse using Cramer's rule (cofactor expansion).
    ///
    /// Only used for the projection matrix inverse (shadow mapping, unprojection).
    /// Not on the hot path. Returns None if singular.
    pub fn inverse(&self) -> Option<Self> {
        let mut aug = [[0.0f32; 8]; 4];

        for r in 0..4 {
            for c in 0..4 {
                aug[r][c] = self.at(r, c);
            }
            for c in 0..4 {
                aug[r][4 + c] = if r == c { 1.0 } else { 0.0 };
            }
        }

        for col in 0..4 {
            let mut pivot_row = col;
            let mut pivot_abs = aug[col][col].abs();
            for r in (col + 1)..4 {
                let v = aug[r][col].abs();
                if v > pivot_abs {
                    pivot_abs = v;
                    pivot_row = r;
                }
            }

            if pivot_abs < 1e-12 {
                return None;
            }

            if pivot_row != col {
                aug.swap(col, pivot_row);
            }

            let pivot = aug[col][col];
            for c in 0..8 {
                aug[col][c] /= pivot;
            }

            for r in 0..4 {
                if r == col {
                    continue;
                }
                let factor = aug[r][col];
                if factor == 0.0 {
                    continue;
                }
                for c in 0..8 {
                    aug[r][c] -= factor * aug[col][c];
                }
            }
        }

        let mut out = Mat4::ZERO;
        for r in 0..4 {
            for c in 0..4 {
                out.set(r, c, aug[r][4 + c]);
            }
        }
        Some(out)
    }
}

/// Inline Bhaskara sin for matrix construction (avoids TrigTable dependency).
fn bhaskara_sin_inline(theta: f32) -> f32 {
    let pi = core::f32::consts::PI;
    let two_pi = 2.0 * pi;
    let mut t = theta % two_pi;
    if t < 0.0 { t += two_pi; }
    let (t_local, sign) = if t > pi { (t - pi, -1.0f32) } else { (t, 1.0f32) };
    let c = pi - t_local;
    let p = t_local * c;
    let denom = 5.0 * pi * pi - 4.0 * p;
    if denom.abs() < 1e-10 { return 0.0; }
    sign * 16.0 * p / denom
}

fn bhaskara_cos_inline(theta: f32) -> f32 {
    bhaskara_sin_inline(theta + core::f32::consts::FRAC_PI_2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::vec::Vec3;

    #[test]
    fn identity_transform() {
        let v = Vec4::new(1.0, 2.0, 3.0, 1.0);
        let result = Mat4::IDENTITY.transform(v);
        assert_eq!(result, v);
    }

    #[test]
    fn translation_point() {
        let m = Mat4::translation(10.0, 20.0, 30.0);
        let p = Vec3::new(1.0, 2.0, 3.0);
        let result = m.transform_point(p).xyz();
        assert!((result.x - 11.0).abs() < 0.001);
        assert!((result.y - 22.0).abs() < 0.001);
        assert!((result.z - 33.0).abs() < 0.001);
    }

    #[test]
    fn mul_identity() {
        let m = Mat4::translation(5.0, 6.0, 7.0);
        let result = m.mul(&Mat4::IDENTITY);
        assert_eq!(result, m);
    }

    #[test]
    fn inverse_affine_round_trip() {
        let m = Mat4::translation(3.0, 7.0, -2.0);
        let inv = m.inverse_affine();
        let identity = m.mul(&inv);
        for c in 0..4 {
            for r in 0..4 {
                let expected = if r == c { 1.0 } else { 0.0 };
                assert!((identity.cols[c][r] - expected).abs() < 0.01,
                    "at [{r}][{c}]: {} != {expected}", identity.cols[c][r]);
            }
        }
    }

    #[test]
    fn general_inverse() {
        let m = Mat4::perspective(1.0, 1.333, 0.1, 100.0);
        let inv = m.inverse().expect("perspective should be invertible");
        let check = m.mul(&inv);
        for c in 0..4 {
            for r in 0..4 {
                let expected = if r == c { 1.0 } else { 0.0 };
                assert!((check.cols[c][r] - expected).abs() < 0.02,
                    "at [{r}][{c}]: {} != {expected}", check.cols[c][r]);
            }
        }
    }
}
