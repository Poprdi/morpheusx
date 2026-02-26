use alloc::vec::Vec;
use crate::math::vec::{Vec2, Vec3};

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
        Self { vertices, indices, bound_center: center, bound_radius: radius }
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Generate a unit cube centered at origin.
    pub fn cube() -> Self {
        let v = |x: f32, y: f32, z: f32, nx: f32, ny: f32, nz: f32, u: f32, v: f32| {
            MeshVertex {
                position: Vec3::new(x, y, z),
                normal: Vec3::new(nx, ny, nz),
                uv: Vec2::new(u, v),
                color: [255, 255, 255, 255],
            }
        };

        let vertices = alloc::vec![
            // Front face
            v(-0.5, -0.5,  0.5,  0.0,  0.0,  1.0, 0.0, 0.0),
            v( 0.5, -0.5,  0.5,  0.0,  0.0,  1.0, 1.0, 0.0),
            v( 0.5,  0.5,  0.5,  0.0,  0.0,  1.0, 1.0, 1.0),
            v(-0.5,  0.5,  0.5,  0.0,  0.0,  1.0, 0.0, 1.0),
            // Back face
            v( 0.5, -0.5, -0.5,  0.0,  0.0, -1.0, 0.0, 0.0),
            v(-0.5, -0.5, -0.5,  0.0,  0.0, -1.0, 1.0, 0.0),
            v(-0.5,  0.5, -0.5,  0.0,  0.0, -1.0, 1.0, 1.0),
            v( 0.5,  0.5, -0.5,  0.0,  0.0, -1.0, 0.0, 1.0),
            // Top face
            v(-0.5,  0.5,  0.5,  0.0,  1.0,  0.0, 0.0, 0.0),
            v( 0.5,  0.5,  0.5,  0.0,  1.0,  0.0, 1.0, 0.0),
            v( 0.5,  0.5, -0.5,  0.0,  1.0,  0.0, 1.0, 1.0),
            v(-0.5,  0.5, -0.5,  0.0,  1.0,  0.0, 0.0, 1.0),
            // Bottom face
            v(-0.5, -0.5, -0.5,  0.0, -1.0,  0.0, 0.0, 0.0),
            v( 0.5, -0.5, -0.5,  0.0, -1.0,  0.0, 1.0, 0.0),
            v( 0.5, -0.5,  0.5,  0.0, -1.0,  0.0, 1.0, 1.0),
            v(-0.5, -0.5,  0.5,  0.0, -1.0,  0.0, 0.0, 1.0),
            // Right face
            v( 0.5, -0.5,  0.5,  1.0,  0.0,  0.0, 0.0, 0.0),
            v( 0.5, -0.5, -0.5,  1.0,  0.0,  0.0, 1.0, 0.0),
            v( 0.5,  0.5, -0.5,  1.0,  0.0,  0.0, 1.0, 1.0),
            v( 0.5,  0.5,  0.5,  1.0,  0.0,  0.0, 0.0, 1.0),
            // Left face
            v(-0.5, -0.5, -0.5, -1.0,  0.0,  0.0, 0.0, 0.0),
            v(-0.5, -0.5,  0.5, -1.0,  0.0,  0.0, 1.0, 0.0),
            v(-0.5,  0.5,  0.5, -1.0,  0.0,  0.0, 1.0, 1.0),
            v(-0.5,  0.5, -0.5, -1.0,  0.0,  0.0, 0.0, 1.0),
        ];

        let indices = alloc::vec![
            0,  1,  2,  0,  2,  3,   // front
            4,  5,  6,  4,  6,  7,   // back
            8,  9,  10, 8,  10, 11,  // top
            12, 13, 14, 12, 14, 15,  // bottom
            16, 17, 18, 16, 18, 19,  // right
            20, 21, 22, 20, 22, 23,  // left
        ];

        Self::new(vertices, indices)
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
fn compute_bounding_sphere(verts: &[MeshVertex]) -> (Vec3, f32) {
    if verts.is_empty() { return (Vec3::ZERO, 0.0); }

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
        if dist_sq > max_dist_sq { max_dist_sq = dist_sq; }
    }

    let radius = if max_dist_sq > 0.0 {
        max_dist_sq * crate::math::fast::inv_sqrt(max_dist_sq)
    } else {
        0.0
    };

    (center, radius)
}
