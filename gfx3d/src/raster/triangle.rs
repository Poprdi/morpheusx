use crate::math::vec::{Vec2, Vec3, Vec4};

/// A processed vertex ready for rasterization.
///
/// All attributes are in screen-space (post-perspective-divide, post-viewport).
/// The `inv_w` field stores 1/w from clip space for perspective-correct interpolation.
///
/// Why perspective-correct interpolation matters:
/// Quake 1 used affine texture mapping (linear in screen space) which causes swimming.
/// We interpolate attr/w across the scanline, then multiply by w at each pixel.
/// The clever trick: we only compute 1/w per pixel (one multiply), not true division.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Vertex {
    pub pos: Vec4,      // screen-space: x, y are pixel coords; z is depth [0,1]; w = 1/clip_w
    pub color: [f32; 3], // vertex color (Gouraud lighting result), pre-divided by clip_w
    pub uv: Vec2,        // texture coords, pre-divided by clip_w
    pub normal: Vec3,    // world-space normal (for per-pixel effects if budget allows)
    pub world_z: f32,    // world-space distance from camera (for fog)
}

impl Vertex {
    pub const ZEROED: Self = Self {
        pos: Vec4::ZERO,
        color: [0.0, 0.0, 0.0],
        uv: Vec2::ZERO,
        normal: Vec3::ZERO,
        world_z: 0.0,
    };

    /// Linearly interpolate all attributes (used by clipper to produce new vertices).
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

/// Three vertices forming a triangle.
pub struct Triangle {
    pub v: [Vertex; 3],
}

impl Triangle {
    /// Signed area × 2 in screen space (positive = CCW = front-facing).
    ///
    /// This is the cross product of edge vectors (v1-v0) × (v2-v0).
    /// Used for both back-face culling AND as the barycentric denominator.
    #[inline]
    pub fn signed_area_2x(&self) -> f32 {
        let e1x = self.v[1].pos.x - self.v[0].pos.x;
        let e1y = self.v[1].pos.y - self.v[0].pos.y;
        let e2x = self.v[2].pos.x - self.v[0].pos.x;
        let e2y = self.v[2].pos.y - self.v[0].pos.y;
        e1x * e2y - e1y * e2x
    }

    /// Returns true if triangle faces the camera (CCW winding = front).
    #[inline]
    pub fn is_front_facing(&self) -> bool {
        self.signed_area_2x() > 0.0
    }

    /// Bounding box in pixel coordinates, clamped to viewport.
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

        if x0 >= x1 || y0 >= y1 { None } else { Some((x0, y0, x1, y1)) }
    }
}
