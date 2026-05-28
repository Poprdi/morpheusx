use super::{Mesh, MeshVertex};
use crate::math::vec::{Vec2, Vec3};

pub fn plane(width: f32, height: f32, segments_x: usize, segments_z: usize) -> Mesh {
    let segments_x = segments_x.max(1);
    let segments_z = segments_z.max(1);

    let mut vertices = alloc::vec![];
    let mut indices = alloc::vec![];

    let half_w = width * 0.5;
    let half_h = height * 0.5;

    for z in 0..=segments_z {
        let z_f = z as f32 / segments_z as f32;
        let z_pos = -half_h + z_f * height;

        for x in 0..=segments_x {
            let x_f = x as f32 / segments_x as f32;
            let x_pos = -half_w + x_f * width;

            vertices.push(MeshVertex {
                position: Vec3::new(x_pos, 0.0, z_pos),
                normal: Vec3::new(0.0, 1.0, 0.0),
                uv: Vec2::new(x_f, z_f),
                color: [255, 255, 255, 255],
            });
        }
    }

    for z in 0..segments_z {
        for x in 0..segments_x {
            let a = z * (segments_x + 1) + x;
            let b = a + 1;
            let c = a + segments_x + 1;
            let d = c + 1;

            if let (Ok(av), Ok(bv), Ok(cv), Ok(dv)) = (
                u16::try_from(a),
                u16::try_from(b),
                u16::try_from(c),
                u16::try_from(d),
            ) {
                indices.push(av);
                indices.push(cv);
                indices.push(bv);

                indices.push(bv);
                indices.push(cv);
                indices.push(dv);
            }
        }
    }

    Mesh::new(vertices, indices)
}
