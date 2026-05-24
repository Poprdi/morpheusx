use super::{Mesh, MeshVertex};
use crate::math::vec::{Vec2, Vec3};

/// Unit square pyramid: base in XZ plane at y=-0.5, apex at (0, 0.5, 0).
pub fn pyramid() -> Mesh {
    let v = |x: f32, y: f32, z: f32, nx: f32, ny: f32, nz: f32, u: f32, v: f32| MeshVertex {
        position: Vec3::new(x, y, z),
        normal: Vec3::new(nx, ny, nz).normalize(),
        uv: Vec2::new(u, v),
        color: [255, 255, 255, 255],
    };

    let vertices = alloc::vec![
        // 0..3: base
        v(-0.5, -0.5, -0.5, 0.0, -1.0, 0.0, 0.0, 0.0),
        v(0.5, -0.5, -0.5, 0.0, -1.0, 0.0, 1.0, 0.0),
        v(0.5, -0.5, 0.5, 0.0, -1.0, 0.0, 1.0, 1.0),
        v(-0.5, -0.5, 0.5, 0.0, -1.0, 0.0, 0.0, 1.0),
        // 4: apex
        v(0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0),
    ];

    let indices = alloc::vec![
        0, 1, 2, 0, 2, 3, // base
        0, 4, 1, // sides
        1, 4, 2, 2, 4, 3, 3, 4, 0,
    ];

    Mesh::new(vertices, indices)
}
