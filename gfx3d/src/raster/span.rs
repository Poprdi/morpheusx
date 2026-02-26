use crate::math::fixed::Fx16;
use super::triangle::Vertex;

/// A horizontal span (scanline segment) produced by the edge walker.
///
/// The edge function approach: we walk left and right edges top-to-bottom,
/// producing one Span per scanline. Each span stores the start/end x and
/// interpolated attributes at both endpoints. The inner loop then steps
/// between them using fixed-point increments.
///
/// This is the Quake 1/2 approach: edge → span → pixel. Modern GPUs
/// use tile-based rasterization, but for software rendering, span-based
/// is still king because it's perfectly cache-friendly (sequential writes
/// to the framebuffer row).
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub y: u32,
    pub x_left: Fx16,
    pub x_right: Fx16,

    // Interpolated attributes at left edge (all pre-divided by w for perspective correction)
    pub inv_w_left: Fx16,
    pub r_left: Fx16,
    pub g_left: Fx16,
    pub b_left: Fx16,
    pub u_left: Fx16,
    pub v_left: Fx16,
    pub z_left: Fx16,
    pub fog_left: Fx16,
}

impl Span {
    pub const EMPTY: Self = Self {
        y: 0,
        x_left: Fx16::ZERO, x_right: Fx16::ZERO,
        inv_w_left: Fx16::ZERO,
        r_left: Fx16::ZERO,
        g_left: Fx16::ZERO,
        b_left: Fx16::ZERO,
        u_left: Fx16::ZERO,
        v_left: Fx16::ZERO,
        z_left: Fx16::ZERO,
        fog_left: Fx16::ZERO,
    };

    /// Width of this span in pixels.
    #[inline]
    pub fn width(&self) -> i32 {
        self.x_right.ceil() - self.x_left.ceil()
    }
}

/// Compute per-pixel step increments for a span.
///
/// This pre-computes 1/(x_right - x_left) once, then multiplies each
/// attribute delta by it. The inner pixel loop only needs additions.
#[derive(Debug, Clone, Copy)]
pub struct SpanGradients {
    pub inv_w_step: Fx16,
    pub r_step: Fx16,
    pub g_step: Fx16,
    pub b_step: Fx16,
    pub u_step: Fx16,
    pub v_step: Fx16,
    pub z_step: Fx16,
    pub fog_step: Fx16,
}

impl SpanGradients {
    pub fn from_triangle(v0: &Vertex, v1: &Vertex, v2: &Vertex) -> Self {
        let dx1 = v1.pos.x - v0.pos.x;
        let dy1 = v1.pos.y - v0.pos.y;
        let dx2 = v2.pos.x - v0.pos.x;
        let dy2 = v2.pos.y - v0.pos.y;

        let area = dx1 * dy2 - dx2 * dy1;
        if area.abs() < 0.0001 {
            return Self {
                inv_w_step: Fx16::ZERO, r_step: Fx16::ZERO, g_step: Fx16::ZERO,
                b_step: Fx16::ZERO, u_step: Fx16::ZERO, v_step: Fx16::ZERO,
                z_step: Fx16::ZERO, fog_step: Fx16::ZERO,
            };
        }

        let inv_area = 1.0 / area;

        let calc_step = |a0: f32, a1: f32, a2: f32| -> Fx16 {
            let da1 = a1 - a0;
            let da2 = a2 - a0;
            let step_x = (da1 * dy2 - da2 * dy1) * inv_area;
            Fx16::from_f32(step_x)
        };

        Self {
            inv_w_step: calc_step(v0.pos.w, v1.pos.w, v2.pos.w),
            r_step: calc_step(v0.color[0], v1.color[0], v2.color[0]),
            g_step: calc_step(v0.color[1], v1.color[1], v2.color[1]),
            b_step: calc_step(v0.color[2], v1.color[2], v2.color[2]),
            u_step: calc_step(v0.uv.x, v1.uv.x, v2.uv.x),
            v_step: calc_step(v0.uv.y, v1.uv.y, v2.uv.y),
            z_step: calc_step(v0.pos.z, v1.pos.z, v2.pos.z),
            fog_step: calc_step(v0.world_z, v1.world_z, v2.world_z),
        }
    }
}
