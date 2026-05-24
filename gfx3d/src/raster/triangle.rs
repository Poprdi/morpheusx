use crate::math::vec::{Vec2, Vec3, Vec4};

/// Screen-space vertex (post-perspective-divide). Attributes pre-divided by clip_w
/// for perspective-correct interpolation; pos.w holds 1/clip_w.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Vertex {
    pub pos: Vec4,
    pub color: [f32; 3],
    pub uv: Vec2,
    pub normal: Vec3,
    pub world_z: f32, // for fog
}

impl Vertex {
    pub const ZEROED: Self = Self {
        pos: Vec4::ZERO,
        color: [0.0, 0.0, 0.0],
        uv: Vec2::ZERO,
        normal: Vec3::ZERO,
        world_z: 0.0,
    };

    #[inline]
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            pos: self.pos.lerp(other.pos, t),
            color: [
                self.color[0] + (other.color[0] - self.color[0]) * t,
                self.color[1] + (other.color[1] - self.color[1]) * t,
                self.color[2] + (other.color[2] - self.color[2]) * t,
            ],
            uv: self.uv.lerp(other.uv, t),
            normal: self.normal.lerp(other.normal, t),
            world_z: self.world_z + (other.world_z - self.world_z) * t,
        }
    }
}

pub struct Triangle {
    pub v: [Vertex; 3],
}

impl Triangle {
    /// 2x signed screen-space area; positive = CCW = front. Doubles as barycentric denominator.
    #[inline]
    pub fn signed_area_2x(&self) -> f32 {
        let e1x = self.v[1].pos.x - self.v[0].pos.x;
        let e1y = self.v[1].pos.y - self.v[0].pos.y;
        let e2x = self.v[2].pos.x - self.v[0].pos.x;
        let e2y = self.v[2].pos.y - self.v[0].pos.y;
        e1x * e2y - e1y * e2x
    }

    #[inline]
    pub fn is_front_facing(&self) -> bool {
        self.signed_area_2x() > 0.0
    }

    /// Pixel-space AABB clamped to viewport.
    #[inline]
    pub fn screen_bounds(&self, vp_w: u32, vp_h: u32) -> Option<(u32, u32, u32, u32)> {
        let min_x = self.v[0].pos.x.min(self.v[1].pos.x).min(self.v[2].pos.x);
        let max_x = self.v[0].pos.x.max(self.v[1].pos.x).max(self.v[2].pos.x);
        let min_y = self.v[0].pos.y.min(self.v[1].pos.y).min(self.v[2].pos.y);
        let max_y = self.v[0].pos.y.max(self.v[1].pos.y).max(self.v[2].pos.y);

        let x0 = (min_x as i32).max(0) as u32;
        let y0 = (min_y as i32).max(0) as u32;
        let x1 = ((max_x as i32) + 1).max(0).min(vp_w as i32) as u32;
        let y1 = ((max_y as i32) + 1).max(0).min(vp_h as i32) as u32;

        if x0 >= x1 || y0 >= y1 {
            None
        } else {
            Some((x0, y0, x1, y1))
        }
    }
}
