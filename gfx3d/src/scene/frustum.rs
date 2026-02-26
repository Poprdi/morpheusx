use crate::math::vec::{Vec3, Vec4};
use crate::math::mat4::Mat4;

/// View frustum for fast culling.
///
/// Extracted from the combined projection × view matrix. Each of the 6 frustum
/// planes is stored as a Vec4 (normal.xyz, distance.w) in world space.
///
/// Plane extraction trick (Gribb & Hartmann, 2001):
/// Given clip matrix M, the 6 planes are:
///   Left:   row3 + row0
///   Right:  row3 - row0
///   Bottom: row3 + row1
///   Top:    row3 - row1
///   Near:   row2         (reversed-Z: near = z ≥ 0 in clip)
///   Far:    row3 - row2  (reversed-Z: far = w - z ≥ 0)
///
/// This extracts planes directly from the matrix without needing to
/// decompose it — works for any projection (perspective, ortho, oblique).
pub struct Frustum {
    pub planes: [Vec4; 6], // (nx, ny, nz, d) — point-on-positive-side = inside
}

impl Frustum {
    /// Extract frustum planes from combined (projection × view) matrix.
    pub fn from_view_proj(vp: &Mat4) -> Self {
        // Row extraction: vp.cols[c][r] means column c, row r.
        // Row i of the matrix = [cols[0][i], cols[1][i], cols[2][i], cols[3][i]]
        let row = |i: usize| -> Vec4 {
            Vec4::new(vp.cols[0][i], vp.cols[1][i], vp.cols[2][i], vp.cols[3][i])
        };

        let r0 = row(0);
        let r1 = row(1);
        let r2 = row(2);
        let r3 = row(3);

        let mut planes = [
            r3 + r0, // left
            r3 - r0, // right
            r3 + r1, // bottom
            r3 - r1, // top
            r2,      // near (reversed-Z)
            r3 - r2, // far (reversed-Z)
        ];

        // Normalize planes (so distance tests give true Euclidean distance)
        for p in &mut planes {
            let len = Vec3::new(p.x, p.y, p.z).length_sq();
            if len > 1e-10 {
                let inv = crate::math::fast::inv_sqrt(len);
                *p = *p * inv;
            }
        }

        Self { planes }
    }

    /// Test a bounding sphere against the frustum.
    ///
    /// Returns:
    /// - `CullResult::Outside` if the sphere is fully outside any plane
    /// - `CullResult::Inside` if fully inside all planes
    /// - `CullResult::Intersect` if partially visible
    ///
    /// This is THE hot-path culling test. Called once per mesh per frame.
    /// A well-authored scene with 500 meshes and good PVS will still test
    /// ~100 spheres. At 60 FPS that's 6000 sphere-frustum tests/sec — trivial.
    #[inline]
    pub fn test_sphere(&self, center: Vec3, radius: f32) -> CullResult {
        let mut all_inside = true;

        for plane in &self.planes {
            let dist = plane.x * center.x + plane.y * center.y + plane.z * center.z + plane.w;
            if dist < -radius {
                return CullResult::Outside;
            }
            if dist < radius {
                all_inside = false;
            }
        }

        if all_inside { CullResult::Inside } else { CullResult::Intersect }
    }

    /// Test an AABB against the frustum.
    ///
    /// Uses the "positive vertex" optimization: for each plane, test only
    /// the corner of the AABB that is most in the direction of the plane
    /// normal. If that corner is outside, the entire box is outside.
    #[inline]
    pub fn test_aabb(&self, min: Vec3, max: Vec3) -> CullResult {
        let mut all_inside = true;

        for plane in &self.planes {
            // Positive vertex: the corner of the AABB furthest along the plane normal
            let px = if plane.x >= 0.0 { max.x } else { min.x };
            let py = if plane.y >= 0.0 { max.y } else { min.y };
            let pz = if plane.z >= 0.0 { max.z } else { min.z };

            let p_dist = plane.x * px + plane.y * py + plane.z * pz + plane.w;
            if p_dist < 0.0 {
                return CullResult::Outside;
            }

            // Negative vertex: opposite corner
            let nx = if plane.x >= 0.0 { min.x } else { max.x };
            let ny = if plane.y >= 0.0 { min.y } else { max.y };
            let nz = if plane.z >= 0.0 { min.z } else { max.z };

            let n_dist = plane.x * nx + plane.y * ny + plane.z * nz + plane.w;
            if n_dist < 0.0 {
                all_inside = false;
            }
        }

        if all_inside { CullResult::Inside } else { CullResult::Intersect }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullResult {
    Outside,
    Inside,
    Intersect,
}
