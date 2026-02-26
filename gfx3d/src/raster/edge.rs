use crate::math::fixed::Fx16;
use super::span::Span;
use super::triangle::Vertex;

/// no_std ceil for f32.
#[inline(always)]
fn float_ceil(x: f32) -> i32 {
    let i = x as i32;
    if (i as f32) < x { i + 1 } else { i }
}

/// DDA edge stepper for triangle rasterization.
///
/// Digital Differential Analyzer: steps an edge from vertex A to vertex B
/// one scanline at a time, producing interpolated attribute values.
///
/// This is the same approach as Quake 1's r_edge.c but with perspective-correct
/// attributes (pre-divided by w) and fixed-point stepping for zero FPU traffic
/// in the inner loop.
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub y_start: i32,
    pub y_end: i32,
    pub x: Fx16,
    pub x_step: Fx16,

    pub inv_w: Fx16,
    pub inv_w_step: Fx16,

    pub r: Fx16, pub r_step: Fx16,
    pub g: Fx16, pub g_step: Fx16,
    pub b: Fx16, pub b_step: Fx16,

    pub u: Fx16, pub u_step: Fx16,
    pub v: Fx16, pub v_step: Fx16,

    pub z: Fx16, pub z_step: Fx16,

    pub fog: Fx16, pub fog_step: Fx16,
}

impl Edge {
    /// Build edge from vertex A (top) to vertex B (bottom).
    ///
    /// Pre-computes per-scanline stepping increments for all attributes.
    /// Uses a single fixed-point reciprocal of dy to compute all steps.
    pub fn new(a: &Vertex, b: &Vertex) -> Self {
        let y_start = float_ceil(a.pos.y);
        let y_end = float_ceil(b.pos.y);
        let dy_f = b.pos.y - a.pos.y;

        if dy_f.abs() < 0.001 || y_start >= y_end {
            return Self::degenerate(y_start, y_end, a);
        }

        let inv_dy = 1.0 / dy_f;
        let prestep = y_start as f32 - a.pos.y;

        let x_step_f = (b.pos.x - a.pos.x) * inv_dy;
        let x = Fx16::from_f32(a.pos.x + prestep * x_step_f);
        let x_step = Fx16::from_f32(x_step_f);

        // All attributes are pre-divided by clip_w in the vertex
        let inv_w_a = a.pos.w;
        let inv_w_b = b.pos.w;
        let _inv_w_step_f = (inv_w_b - inv_w_a) * inv_dy;

        let lerp_attr = |a_val: f32, b_val: f32| -> (Fx16, Fx16) {
            let step_f = (b_val - a_val) * inv_dy;
            (Fx16::from_f32(a_val + prestep * step_f), Fx16::from_f32(step_f))
        };

        let (inv_w, inv_w_step) = lerp_attr(inv_w_a, inv_w_b);
        let (r, r_step) = lerp_attr(a.color[0], b.color[0]);
        let (g, g_step) = lerp_attr(a.color[1], b.color[1]);
        let (b_attr, b_step) = lerp_attr(a.color[2], b.color[2]);
        let (u, u_step) = lerp_attr(a.uv.x, b.uv.x);
        let (v, v_step) = lerp_attr(a.uv.y, b.uv.y);
        let (z, z_step) = lerp_attr(a.pos.z, b.pos.z);
        let (fog, fog_step) = lerp_attr(a.world_z, b.world_z);

        Self {
            y_start, y_end, x, x_step,
            inv_w, inv_w_step,
            r, r_step, g, g_step, b: b_attr, b_step,
            u, u_step, v, v_step,
            z, z_step, fog, fog_step,
        }
    }

    fn degenerate(y_start: i32, y_end: i32, v: &Vertex) -> Self {
        Self {
            y_start, y_end,
            x: Fx16::from_f32(v.pos.x), x_step: Fx16::ZERO,
            inv_w: Fx16::from_f32(v.pos.w), inv_w_step: Fx16::ZERO,
            r: Fx16::from_f32(v.color[0]), r_step: Fx16::ZERO,
            g: Fx16::from_f32(v.color[1]), g_step: Fx16::ZERO,
            b: Fx16::from_f32(v.color[2]), b_step: Fx16::ZERO,
            u: Fx16::from_f32(v.uv.x), u_step: Fx16::ZERO,
            v: Fx16::from_f32(v.uv.y), v_step: Fx16::ZERO,
            z: Fx16::from_f32(v.pos.z), z_step: Fx16::ZERO,
            fog: Fx16::from_f32(v.world_z), fog_step: Fx16::ZERO,
        }
    }

    /// Advance all interpolants by one scanline.
    #[inline(always)]
    pub fn step(&mut self) {
        self.x += self.x_step;
        self.inv_w += self.inv_w_step;
        self.r += self.r_step;
        self.g += self.g_step;
        self.b += self.b_step;
        self.u += self.u_step;
        self.v += self.v_step;
        self.z += self.z_step;
        self.fog += self.fog_step;
    }

    /// Produce a span from left and right edges at the current scanline.
    #[inline]
    pub fn make_span(left: &Edge, right: &Edge, y: u32) -> Span {
        Span {
            y,
            x_left: left.x, x_right: right.x,
            inv_w_left: left.inv_w,
            r_left: left.r,
            g_left: left.g,
            b_left: left.b,
            u_left: left.u,
            v_left: left.v,
            z_left: left.z,
            fog_left: left.fog,
        }
    }
}

/// Rasterize a triangle into spans (scanline segments).
///
/// Classic top-down triangle rasterization with long-edge / short-edge split:
/// 1. Sort vertices by y (top to bottom)
/// 2. The longest edge spans the entire triangle height
/// 3. Two shorter edges split at the middle vertex
/// 4. For each scanline, step the long edge and the current short edge
///
/// This produces one Span per visible scanline. The caller then fills each
/// span using the inner pixel loop (span.rs).
///
/// Pre-allocated `spans` slice avoids heap allocation in the render loop.
/// Returns the number of spans written.
pub fn rasterize_triangle_to_spans(
    tri: &[Vertex; 3],
    spans: &mut [Span],
    viewport_h: u32,
) -> usize {
    // Sort by y (insertion sort on 3 elements = branchless-optimal)
    let mut sorted = *tri;
    if sorted[0].pos.y > sorted[1].pos.y { sorted.swap(0, 1); }
    if sorted[1].pos.y > sorted[2].pos.y { sorted.swap(1, 2); }
    if sorted[0].pos.y > sorted[1].pos.y { sorted.swap(0, 1); }

    let v0 = &sorted[0]; // top
    let v1 = &sorted[1]; // middle
    let v2 = &sorted[2]; // bottom

    let total_height = v2.pos.y - v0.pos.y;
    if total_height < 0.5 { return 0; }

    // Long edge: v0 → v2
    let mut long_edge = Edge::new(v0, v2);

    // Determine which side the long edge is on.
    // If mid-vertex is to the LEFT of the long edge, long edge is on the right.
    let mid_x_on_long = v0.pos.x + (v1.pos.y - v0.pos.y) / total_height * (v2.pos.x - v0.pos.x);
    let long_on_right = v1.pos.x < mid_x_on_long;

    let mut count = 0;

    // Top half: v0 → v1
    let mut short = Edge::new(v0, v1);
    let y_start = (short.y_start.max(0) as u32).min(viewport_h);
    let y_mid = (short.y_end.max(0) as u32).min(viewport_h);

    while short.y_start < y_start as i32 && short.y_start < short.y_end {
        short.step();
        short.y_start += 1;
    }
    while long_edge.y_start < y_start as i32 && long_edge.y_start < long_edge.y_end {
        long_edge.step();
        long_edge.y_start += 1;
    }

    for y in y_start..y_mid {
        if count >= spans.len() { break; }
        spans[count] = if long_on_right {
            Edge::make_span(&short, &long_edge, y)
        } else {
            Edge::make_span(&long_edge, &short, y)
        };
        count += 1;
        short.step();
        short.y_start += 1;
        long_edge.step();
        long_edge.y_start += 1;
    }

    // Bottom half: v1 → v2
    let mut short = Edge::new(v1, v2);
    let y_mid2 = (short.y_start.max(0) as u32).min(viewport_h);
    let y_end = (short.y_end.max(0) as u32).min(viewport_h);

    // Sync long edge if we skipped scanlines
    while (long_edge.y_start as u32) < y_mid2 && long_edge.y_start < long_edge.y_end {
        long_edge.step();
        long_edge.y_start += 1;
    }
    while short.y_start < y_mid2 as i32 && short.y_start < short.y_end {
        short.step();
        short.y_start += 1;
    }

    for y in y_mid2..y_end {
        if count >= spans.len() { break; }
        spans[count] = if long_on_right {
            Edge::make_span(&short, &long_edge, y)
        } else {
            Edge::make_span(&long_edge, &short, y)
        };
        count += 1;
        short.step();
        short.y_start += 1;
        long_edge.step();
        long_edge.y_start += 1;
    }

    count
}
