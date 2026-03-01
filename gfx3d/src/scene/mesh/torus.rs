use super::{Mesh, MeshVertex};
use crate::math::trig::TrigTable;
use crate::math::vec::{Vec2, Vec3};

/// Generate a torus (donut shape).
///
/// `major_radius`: distance from center of torus to center of tube
/// `minor_radius`: radius of the tube itself
/// `major_segments`: number of segments around the major ring
/// `minor_segments`: number of segments around the minor circle
pub fn torus(
    major_radius: f32,
    minor_radius: f32,
    major_segments: usize,
    minor_segments: usize,
) -> Mesh {
    let major_segments = major_segments.max(3);
    let minor_segments = minor_segments.max(3);

    let trig = TrigTable::new();
    let mut vertices = alloc::vec![];
    let mut indices = alloc::vec![];

    let two_pi = 2.0 * core::f32::consts::PI;

    // Generate vertices
    for i in 0..major_segments {
        let i_f = i as f32 / major_segments as f32;
        let major_angle = two_pi * i_f;
        let (sin_maj, cos_maj) = trig.sin_cos(major_angle);

        for j in 0..minor_segments {
            let j_f = j as f32 / minor_segments as f32;
            let minor_angle = two_pi * j_f;
            let (sin_min, cos_min) = trig.sin_cos(minor_angle);

            // Position on the major ring
            let ring_x = major_radius * cos_maj;
            let ring_z = major_radius * sin_maj;

            // Position on the minor circle, offset from the ring
            let x = ring_x + minor_radius * cos_min * cos_maj;
            let y = minor_radius * sin_min;
            let z = ring_z + minor_radius * cos_min * sin_maj;

            // Normal points outward from the minor circle
            let nx = cos_min * cos_maj;
            let ny = sin_min;
            let nz = cos_min * sin_maj;

            vertices.push(MeshVertex {
                position: Vec3::new(x, y, z),
                normal: Vec3::new(nx, ny, nz),
                uv: Vec2::new(i_f, j_f),
                color: [255, 255, 255, 255],
            });
        }
    }

    // Generate indices
    for i in 0..major_segments {
        for j in 0..minor_segments {
            let a = (i * minor_segments) + j;
            let b = (i * minor_segments) + ((j + 1) % minor_segments);
            let c = (((i + 1) % major_segments) * minor_segments) + j;
            let d = (((i + 1) % major_segments) * minor_segments) + ((j + 1) % minor_segments);

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
