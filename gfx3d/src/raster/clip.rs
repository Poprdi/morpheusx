use super::triangle::Vertex;
use crate::math::vec::Vec4;

/// Sutherland-Hodgman clipper against the 6 frustum planes in clip space.
///
/// This operates BEFORE perspective divide, in 4D homogeneous coordinates.
/// Each frustum plane is an inequality on clip-space coords:
///   -w ≤ x ≤ w,  -w ≤ y ≤ w,  0 ≤ z ≤ w  (reversed-Z: near=w, far=0)
///
/// The classic Quake approach was to clip in screen space after projection,
/// but clip-space clipping handles the near plane correctly (which is
/// critical — screen-space near-plane clipping requires special-casing).
///
/// We use a fixed-size workspace of 12 vertices (a convex polygon clipped
/// against 6 planes can produce at most 6+3=9 new vertices from a triangle,
/// but 12 gives margin). Zero heap allocation.
pub struct Clipper {
    buf_a: [Vertex; 12],
    buf_b: [Vertex; 12],
}

impl Default for Clipper {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipper {
    pub const fn new() -> Self {
        Self {
            buf_a: [Vertex::ZEROED; 12],
            buf_b: [Vertex::ZEROED; 12],
        }
    }

    /// Clip a triangle against all 6 frustum planes.
    ///
    /// Returns a slice of output vertices forming a convex polygon (fan-ready).
    /// If the triangle is fully outside, returns an empty slice.
    /// Vertices are in clip space (pre-perspective-divide).
    pub fn clip_triangle<'a>(&'a mut self, verts: &[Vertex; 3]) -> &'a [Vertex] {
        self.buf_a[0] = verts[0];
        self.buf_a[1] = verts[1];
        self.buf_a[2] = verts[2];
        let mut count = 3usize;
        let mut src_a = true;

        // Clip against each plane. The planes in clip space:
        // x + w ≥ 0  (left)
        // w - x ≥ 0  (right)
        // y + w ≥ 0  (bottom)
        // w - y ≥ 0  (top)
        // z     ≥ 0  (near, reversed-Z)
        // w - z ≥ 0  (far, reversed-Z)

        macro_rules! clip_plane {
            ($dist_fn:expr) => {{
                let (src, dst) = if src_a {
                    (
                        &self.buf_a as &[Vertex; 12],
                        &mut self.buf_b as &mut [Vertex; 12],
                    )
                } else {
                    (
                        &self.buf_b as &[Vertex; 12],
                        &mut self.buf_a as &mut [Vertex; 12],
                    )
                };
                count = clip_polygon_against_plane(src, count, dst, $dist_fn);
                if count == 0 {
                    return &[];
                }
                src_a = !src_a;
            }};
        }

        clip_plane!(|v: &Vec4| v.x + v.w); // left
        clip_plane!(|v: &Vec4| v.w - v.x); // right
        clip_plane!(|v: &Vec4| v.y + v.w); // bottom
        clip_plane!(|v: &Vec4| v.w - v.y); // top
        clip_plane!(|v: &Vec4| v.z); // near
        clip_plane!(|v: &Vec4| v.w - v.z); // far

        if src_a {
            &self.buf_a[..count]
        } else {
            &self.buf_b[..count]
        }
    }
}

/// Clip a convex polygon against a single plane (Sutherland-Hodgman single pass).
///
/// `dist_fn` returns the signed distance from the plane (positive = inside).
fn clip_polygon_against_plane<F>(
    input: &[Vertex; 12],
    in_count: usize,
    output: &mut [Vertex; 12],
    dist_fn: F,
) -> usize
where
    F: Fn(&Vec4) -> f32,
{
    if in_count == 0 {
        return 0;
    }
    let mut out_count = 0usize;

    let mut prev_idx = in_count - 1;
    let mut prev_dist = dist_fn(&input[prev_idx].pos);

    for curr_idx in 0..in_count {
        let curr_dist = dist_fn(&input[curr_idx].pos);

        if prev_dist >= 0.0 {
            // Previous vertex is inside
            if curr_dist >= 0.0 {
                // Both inside: emit current
                if out_count < 12 {
                    output[out_count] = input[curr_idx];
                    out_count += 1;
                }
            } else {
                // Crossing out: emit intersection
                let t = prev_dist / (prev_dist - curr_dist);
                if out_count < 12 {
                    output[out_count] = input[prev_idx].lerp(&input[curr_idx], t);
                    out_count += 1;
                }
            }
        } else {
            // Previous vertex is outside
            if curr_dist >= 0.0 {
                // Crossing in: emit intersection, then current
                let t = prev_dist / (prev_dist - curr_dist);
                if out_count < 12 {
                    output[out_count] = input[prev_idx].lerp(&input[curr_idx], t);
                    out_count += 1;
                }
                if out_count < 12 {
                    output[out_count] = input[curr_idx];
                    out_count += 1;
                }
            }
            // Both outside: emit nothing
        }

        prev_idx = curr_idx;
        prev_dist = curr_dist;
    }

    out_count
}
