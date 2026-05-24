use super::triangle::Vertex;
use crate::math::vec::Vec4;

/// Sutherland-Hodgman clip in 4D clip space (handles near-plane without special-casing).
/// 12-vertex workspace; triangle vs. 6 planes can yield up to 9.
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

    /// Returns the clipped polygon as a fan-ready slice; empty if fully outside.
    /// Input vertices are pre-perspective-divide (clip space).
    pub fn clip_triangle<'a>(&'a mut self, verts: &[Vertex; 3]) -> &'a [Vertex] {
        self.buf_a[0] = verts[0];
        self.buf_a[1] = verts[1];
        self.buf_a[2] = verts[2];
        let mut count = 3usize;
        let mut src_a = true;

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

        // Reversed-Z clip planes: near = z≥0, far = w-z≥0.
        clip_plane!(|v: &Vec4| v.x + v.w);
        clip_plane!(|v: &Vec4| v.w - v.x);
        clip_plane!(|v: &Vec4| v.y + v.w);
        clip_plane!(|v: &Vec4| v.w - v.y);
        clip_plane!(|v: &Vec4| v.z);
        clip_plane!(|v: &Vec4| v.w - v.z);

        if src_a {
            &self.buf_a[..count]
        } else {
            &self.buf_b[..count]
        }
    }
}

/// `dist_fn` is signed distance to the plane; positive = inside.
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
            if curr_dist >= 0.0 {
                if out_count < 12 {
                    output[out_count] = input[curr_idx];
                    out_count += 1;
                }
            } else {
                let t = prev_dist / (prev_dist - curr_dist);
                if out_count < 12 {
                    output[out_count] = input[prev_idx].lerp(&input[curr_idx], t);
                    out_count += 1;
                }
            }
        } else if curr_dist >= 0.0 {
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

        prev_idx = curr_idx;
        prev_dist = curr_dist;
    }

    out_count
}
