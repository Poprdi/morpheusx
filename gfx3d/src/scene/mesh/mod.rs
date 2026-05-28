use crate::math::vec::{Vec2, Vec3};
use alloc::vec::Vec;

pub mod cube;
pub mod cylinder;
pub mod plane;
pub mod pyramid;
pub mod sphere;
pub mod torus;

/// Model-space vertex.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MeshVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub color: [u8; 4],
}

/// Indexed triangle mesh. Interleaved verts + u16 indices (65K-vert cap per mesh).
pub struct Mesh {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u16>,
    /// Bounding sphere for frustum rejection.
    pub bound_center: Vec3,
    pub bound_radius: f32,
}

impl Mesh {
    pub fn new(vertices: Vec<MeshVertex>, indices: Vec<u16>) -> Self {
        let (center, radius) = compute_bounding_sphere(&vertices);
        Self {
            vertices,
            indices,
            bound_center: center,
            bound_radius: radius,
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn cube() -> Self {
        cube::cube()
    }

    /// UV sphere (lat-long parameterization).
    pub fn sphere(stacks: usize, slices: usize) -> Self {
        sphere::sphere(stacks, slices)
    }

    pub fn torus(
        major_radius: f32,
        minor_radius: f32,
        major_segments: usize,
        minor_segments: usize,
    ) -> Self {
        torus::torus(major_radius, minor_radius, major_segments, minor_segments)
    }

    pub fn pyramid() -> Self {
        pyramid::pyramid()
    }

    pub fn plane(width: f32, height: f32, segments_x: usize, segments_z: usize) -> Self {
        plane::plane(width, height, segments_x, segments_z)
    }

    pub fn cylinder(radius: f32, height: f32, segments: usize) -> Self {
        cylinder::cylinder(radius, height, segments)
    }

    /// Call after mutating vertices.
    pub fn recompute_bounds(&mut self) {
        let (c, r) = compute_bounding_sphere(&self.vertices);
        self.bound_center = c;
        self.bound_radius = r;
    }
}

pub fn compute_bounding_sphere(verts: &[MeshVertex]) -> (Vec3, f32) {
    if verts.is_empty() {
        return (Vec3::ZERO, 0.0);
    }

    let mut min = verts[0].position;
    let mut max = verts[0].position;
    for v in verts.iter().skip(1) {
        min = min.min(v.position);
        max = max.max(v.position);
    }
    let center = (min + max) * 0.5;

    let mut max_dist_sq = 0.0f32;
    for v in verts {
        let diff = v.position - center;
        let dist_sq = diff.length_sq();
        if dist_sq > max_dist_sq {
            max_dist_sq = dist_sq;
        }
    }

    let radius = if max_dist_sq > 0.0 {
        max_dist_sq * crate::math::fast::inv_sqrt(max_dist_sq)
    } else {
        0.0
    };

    (center, radius)
}
