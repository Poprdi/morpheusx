use super::{Mesh, MeshVertex};
use crate::math::vec::{Vec2, Vec3};

/// Generate a unit cube centered at origin.
pub fn cube() -> Mesh {
    let v = |x: f32, y: f32, z: f32, nx: f32, ny: f32, nz: f32, u: f32, v: f32| MeshVertex {
        position: Vec3::new(x, y, z),
        normal: Vec3::new(nx, ny, nz),
        uv: Vec2::new(u, v),
        color: [255, 255, 255, 255],
    };

    let vertices = alloc::vec![
        // Front face
        v(-0.5, -0.5, 0.5, 0.0, 0.0, 1.0, 0.0, 0.0),
        v(0.5, -0.5, 0.5, 0.0, 0.0, 1.0, 1.0, 0.0),
        v(0.5, 0.5, 0.5, 0.0, 0.0, 1.0, 1.0, 1.0),
        v(-0.5, 0.5, 0.5, 0.0, 0.0, 1.0, 0.0, 1.0),
        // Back face
        v(0.5, -0.5, -0.5, 0.0, 0.0, -1.0, 0.0, 0.0),
        v(-0.5, -0.5, -0.5, 0.0, 0.0, -1.0, 1.0, 0.0),
        v(-0.5, 0.5, -0.5, 0.0, 0.0, -1.0, 1.0, 1.0),
        v(0.5, 0.5, -0.5, 0.0, 0.0, -1.0, 0.0, 1.0),
        // Top face
        v(-0.5, 0.5, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0),
        v(0.5, 0.5, 0.5, 0.0, 1.0, 0.0, 1.0, 0.0),
        v(0.5, 0.5, -0.5, 0.0, 1.0, 0.0, 1.0, 1.0),
        v(-0.5, 0.5, -0.5, 0.0, 1.0, 0.0, 0.0, 1.0),
        // Bottom face
        v(-0.5, -0.5, -0.5, 0.0, -1.0, 0.0, 0.0, 0.0),
        v(0.5, -0.5, -0.5, 0.0, -1.0, 0.0, 1.0, 0.0),
        v(0.5, -0.5, 0.5, 0.0, -1.0, 0.0, 1.0, 1.0),
        v(-0.5, -0.5, 0.5, 0.0, -1.0, 0.0, 0.0, 1.0),
        // Right face
        v(0.5, -0.5, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0),
        v(0.5, -0.5, -0.5, 1.0, 0.0, 0.0, 1.0, 0.0),
        v(0.5, 0.5, -0.5, 1.0, 0.0, 0.0, 1.0, 1.0),
        v(0.5, 0.5, 0.5, 1.0, 0.0, 0.0, 0.0, 1.0),
        // Left face
        v(-0.5, -0.5, -0.5, -1.0, 0.0, 0.0, 0.0, 0.0),
        v(-0.5, -0.5, 0.5, -1.0, 0.0, 0.0, 1.0, 0.0),
        v(-0.5, 0.5, 0.5, -1.0, 0.0, 0.0, 1.0, 1.0),
        v(-0.5, 0.5, -0.5, -1.0, 0.0, 0.0, 0.0, 1.0),
    ];

    let indices = alloc::vec![
        0, 1, 2, 0, 2, 3, // front
        4, 5, 6, 4, 6, 7, // back
        8, 9, 10, 8, 10, 11, // top
        12, 13, 14, 12, 14, 15, // bottom
        16, 17, 18, 16, 18, 19, // right
        20, 21, 22, 20, 22, 23, // left
    ];

    Mesh::new(vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::mat4::Mat4;
    use crate::math::trig::TrigTable;
    use crate::math::vec::Vec3;

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    fn edge_len_sq(a: Vec3, b: Vec3) -> f32 {
        let d = a - b;
        d.length_sq()
    }

    #[test]
    fn cube_mesh_has_expected_corners() {
        let mesh = cube();
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);

        let mut unique: [Vec3; 8] = [Vec3::ZERO; 8];
        let mut unique_count = 0usize;

        for v in &mesh.vertices {
            assert!(approx_eq(v.position.x.abs(), 0.5, 1e-6));
            assert!(approx_eq(v.position.y.abs(), 0.5, 1e-6));
            assert!(approx_eq(v.position.z.abs(), 0.5, 1e-6));

            let mut found = false;
            for u in unique.iter().take(unique_count) {
                if approx_eq(u.x, v.position.x, 1e-6)
                    && approx_eq(u.y, v.position.y, 1e-6)
                    && approx_eq(u.z, v.position.z, 1e-6)
                {
                    found = true;
                    break;
                }
            }

            if !found && unique_count < unique.len() {
                unique[unique_count] = v.position;
                unique_count += 1;
            }
        }

        assert_eq!(
            unique_count, 8,
            "cube should have exactly 8 unique corner positions"
        );
    }

    #[test]
    fn cube_rotation_and_uniform_scale_are_rigid() {
        let corners = [
            Vec3::new(-0.5, -0.5, -0.5), // 0
            Vec3::new(0.5, -0.5, -0.5),  // 1
            Vec3::new(0.5, 0.5, -0.5),   // 2
            Vec3::new(-0.5, 0.5, -0.5),  // 3
            Vec3::new(-0.5, -0.5, 0.5),  // 4
            Vec3::new(0.5, -0.5, 0.5),   // 5
            Vec3::new(0.5, 0.5, 0.5),    // 6
            Vec3::new(-0.5, 0.5, 0.5),   // 7
        ];

        let edges: [(usize, usize); 12] = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ];

        let trig = TrigTable::new();
        let scale_val = 0.95f32;

        for step in 0..360u32 {
            let angle = (step as f32) * (core::f32::consts::PI / 180.0);
            let (s, c) = trig.sin_cos(angle);
            let transform =
                Mat4::rotation_y(s, c).mul(&Mat4::scale(scale_val, scale_val, scale_val));

            let mut transformed = [Vec3::ZERO; 8];
            for (idx, p) in corners.iter().enumerate() {
                transformed[idx] = transform.transform_point(*p).xyz();
            }

            for (a, b) in edges {
                let len_sq = edge_len_sq(transformed[a], transformed[b]);
                let expected = scale_val * scale_val;
                assert!(
                    approx_eq(len_sq, expected, 0.003),
                    "edge length changed at step {step}: got {len_sq}, expected {expected}"
                );
            }
        }
    }
}
