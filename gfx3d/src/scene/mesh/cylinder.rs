use super::{Mesh, MeshVertex};
use crate::math::trig::TrigTable;
use crate::math::vec::{Vec2, Vec3};

/// Generate a capped cylinder.
///
/// `radius`: radius of the cylinder
/// `height`: total height (from -height/2 to +height/2)
/// `segments`: number of segments around the circumference
pub fn cylinder(radius: f32, height: f32, segments: usize) -> Mesh {
    let segments = segments.max(3);

    let trig = TrigTable::new();
    let mut vertices = alloc::vec![];
    let mut indices = alloc::vec![];

    let two_pi = 2.0 * core::f32::consts::PI;
    let half_h = height * 0.5;

    // Generate side vertices
    for i in 0..=segments {
        let i_f = i as f32 / segments as f32;
        let angle = two_pi * i_f;
        let (sin_a, cos_a) = trig.sin_cos(angle);

        let x = radius * cos_a;
        let z = radius * sin_a;

        // Top vertex
        vertices.push(MeshVertex {
            position: Vec3::new(x, half_h, z),
            normal: Vec3::new(cos_a, 0.0, sin_a),
            uv: Vec2::new(i_f, 1.0),
            color: [255, 255, 255, 255],
        });

        // Bottom vertex
        vertices.push(MeshVertex {
            position: Vec3::new(x, -half_h, z),
            normal: Vec3::new(cos_a, 0.0, sin_a),
            uv: Vec2::new(i_f, 0.0),
            color: [255, 255, 255, 255],
        });
    }

    // Center vertices for caps
    let top_center = vertices.len();
    vertices.push(MeshVertex {
        position: Vec3::new(0.0, half_h, 0.0),
        normal: Vec3::new(0.0, 1.0, 0.0),
        uv: Vec2::new(0.5, 0.5),
        color: [255, 255, 255, 255],
    });

    let bottom_center = vertices.len();
    vertices.push(MeshVertex {
        position: Vec3::new(0.0, -half_h, 0.0),
        normal: Vec3::new(0.0, -1.0, 0.0),
        uv: Vec2::new(0.5, 0.5),
        color: [255, 255, 255, 255],
    });

    // Side indices (quads as two triangles)
    for i in 0..segments {
        let top1 = i * 2;
        let bot1 = top1 + 1;
        let top2 = (i + 1) * 2;
        let bot2 = top2 + 1;

        if let (Ok(t1), Ok(b1), Ok(t2), Ok(b2)) = (
            u16::try_from(top1),
            u16::try_from(bot1),
            u16::try_from(top2),
            u16::try_from(bot2),
        ) {
            // First triangle
            indices.push(t1);
            indices.push(t2);
            indices.push(b1);

            // Second triangle
            indices.push(b1);
            indices.push(t2);
            indices.push(b2);
        }
    }

    // Top cap
    for i in 0..segments {
        let a = i * 2;
        let b = (i + 1) * 2;

        if let (Ok(av), Ok(bv), Ok(c)) = (
            u16::try_from(a),
            u16::try_from(b),
            u16::try_from(top_center),
        ) {
            indices.push(c);
            indices.push(bv);
            indices.push(av);
        }
    }

    // Bottom cap
    for i in 0..segments {
        let a = i * 2 + 1;
        let b = (i + 1) * 2 + 1;

        if let (Ok(av), Ok(bv), Ok(c)) = (
            u16::try_from(a),
            u16::try_from(b),
            u16::try_from(bottom_center),
        ) {
            indices.push(c);
            indices.push(av);
            indices.push(bv);
        }
    }

    Mesh::new(vertices, indices)
}
