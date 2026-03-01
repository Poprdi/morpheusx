use super::{Mesh, MeshVertex};
use crate::math::vec::{Vec2, Vec3};

/// Generate a square pyramid.
///
/// Base is a square in the XZ plane, apex at +Y.
/// Radius: square base from -0.5 to 0.5 in XZ.
/// Height: from -0.5 to +0.5 in Y.
pub fn pyramid() -> Mesh {
    let v = |x: f32, y: f32, z: f32, nx: f32, ny: f32, nz: f32, u: f32, v: f32| MeshVertex {
        position: Vec3::new(x, y, z),
        normal: Vec3::new(nx, ny, nz).normalize(),
        uv: Vec2::new(u, v),
        color: [255, 255, 255, 255],
    };

    let vertices = alloc::vec![
        // Base corners (square in XZ plane at y = -0.5)
        v(-0.5, -0.5, -0.5, 0.0, -1.0, 0.0, 0.0, 0.0), // 0: BL front
        v(0.5, -0.5, -0.5, 0.0, -1.0, 0.0, 1.0, 0.0),  // 1: BR front
        v(0.5, -0.5, 0.5, 0.0, -1.0, 0.0, 1.0, 1.0),   // 2: BR back
        v(-0.5, -0.5, 0.5, 0.0, -1.0, 0.0, 0.0, 1.0),  // 3: BL back
        // Apex at (0, 0.5, 0)
        v(0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0), // 4: apex
    ];

    let indices = alloc::vec![
        // Base (square)
        0, 1, 2, 0, 2, 3, // Front face
        0, 4, 1, // Right face
        1, 4, 2, // Back face
        2, 4, 3, // Left face
        3, 4, 0,
    ];

    Mesh::new(vertices, indices)
}
