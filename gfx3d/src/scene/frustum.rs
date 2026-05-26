use crate::math::mat4::Mat4;
use crate::math::vec::{Vec3, Vec4};

/// 6 frustum planes (nx,ny,nz,d) in world space; positive side = inside.
pub struct Frustum {
    pub planes: [Vec4; 6],
}

impl Frustum {
    /// Gribb-Hartmann extraction from projection*view. Near uses row2 (reversed-Z).
    pub fn from_view_proj(vp: &Mat4) -> Self {
        // cols[c][r] = column c, row r.
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
            r3 - r2, // far
        ];

        for p in &mut planes {
            let len = Vec3::new(p.x, p.y, p.z).length_sq();
            if len > 1e-10 {
                let inv = crate::math::fast::inv_sqrt(len);
                *p = *p * inv;
            }
        }

        Self { planes }
    }

    /// Hot path; called per mesh per frame.
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

        if all_inside {
            CullResult::Inside
        } else {
            CullResult::Intersect
        }
    }

    #[inline]
    pub fn test_aabb(&self, min: Vec3, max: Vec3) -> CullResult {
        let mut all_inside = true;

        for plane in &self.planes {
            let px = if plane.x >= 0.0 { max.x } else { min.x };
            let py = if plane.y >= 0.0 { max.y } else { min.y };
            let pz = if plane.z >= 0.0 { max.z } else { min.z };

            let p_dist = plane.x * px + plane.y * py + plane.z * pz + plane.w;
            if p_dist < 0.0 {
                return CullResult::Outside;
            }

            let nx = if plane.x >= 0.0 { min.x } else { max.x };
            let ny = if plane.y >= 0.0 { min.y } else { max.y };
            let nz = if plane.z >= 0.0 { min.z } else { max.z };

            let n_dist = plane.x * nx + plane.y * ny + plane.z * nz + plane.w;
            if n_dist < 0.0 {
                all_inside = false;
            }
        }

        if all_inside {
            CullResult::Inside
        } else {
            CullResult::Intersect
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullResult {
    Outside,
    Inside,
    Intersect,
}
