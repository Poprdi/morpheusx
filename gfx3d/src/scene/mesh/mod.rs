use crate::math::vec::{Vec2, Vec3};
use alloc::vec::Vec;

pub mod cube;
pub mod cylinder;
pub mod plane;
pub mod pyramid;
pub mod sphere;
pub mod torus;

/// A vertex in a mesh (model-space).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MeshVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub color: [u8; 4], // RGBA vertex color (for baked lighting or tinting)
}

/// An indexed triangle mesh.
///
/// This is the fundamental geometric primitive. Meshes are loaded from disk
/// (OBJ, custom binary format) or generated procedurally (cubes, spheres).
///
/// Storage is interleaved vertex + separate index buffer — this is cache-optimal
/// for vertex transformation (all attributes of one vertex are adjacent in memory).
///
/// Triangle indices are u16 for meshes under 65K verts (saves 50% index memory
/// and bandwidth). For larger meshes, use multiple Mesh instances.
pub struct Mesh {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u16>,
    /// Bounding sphere for quick frustum rejection.
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

    /// Generate a unit cube centered at origin.
    pub fn cube() -> Self {
        cube::cube()
    }

    /// Generate a UV sphere (latitude-longitude).
    pub fn sphere(stacks: usize, slices: usize) -> Self {
        sphere::sphere(stacks, slices)
    }

    /// Generate a torus (donut shape).
    pub fn torus(
        major_radius: f32,
        minor_radius: f32,
        major_segments: usize,
        minor_segments: usize,
    ) -> Self {
        torus::torus(major_radius, minor_radius, major_segments, minor_segments)
    }

    /// Generate a square pyramid.
    pub fn pyramid() -> Self {
        pyramid::pyramid()
    }

    /// Generate a flat plane.
    pub fn plane(width: f32, height: f32, segments_x: usize, segments_z: usize) -> Self {
        plane::plane(width, height, segments_x, segments_z)
    }

    /// Generate a capped cylinder.
    pub fn cylinder(radius: f32, height: f32, segments: usize) -> Self {
        cylinder::cylinder(radius, height, segments)
    }

    /// Recompute bounding sphere (call after modifying vertices).
    pub fn recompute_bounds(&mut self) {
        let (c, r) = compute_bounding_sphere(&self.vertices);
        self.bound_center = c;
        self.bound_radius = r;
    }
}

/// Compute bounding sphere using Ritter's algorithm.
///
/// Not optimal but O(n) and within 5% of optimal radius in practice.
/// Good enough for coarse frustum culling.
pub fn compute_bounding_sphere(verts: &[MeshVertex]) -> (Vec3, f32) {
    if verts.is_empty() {
        return (Vec3::ZERO, 0.0);
    }

    // Start with AABB center
    let mut min = verts[0].position;
    let mut max = verts[0].position;
    for v in verts.iter().skip(1) {
        min = min.min(v.position);
        max = max.max(v.position);
    }
    let center = (min + max) * 0.5;

    // Find max distance from center
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
