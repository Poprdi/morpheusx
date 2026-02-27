use super::{Mesh, MeshVertex};
use crate::math::vec::{Vec2, Vec3};
use crate::math::trig::TrigTable;

/// Generate a UV sphere (latitude-longitude).
///
/// `stacks`: number of vertical divisions (default 16 gives good quality)
/// `slices`: number of horizontal divisions (default 32)
///
/// A unit sphere: radius 1.0, centered at origin.
pub fn sphere(stacks: usize, slices: usize) -> Mesh {
    let stacks = stacks.max(3);
    let slices = slices.max(3);
    
    let trig = TrigTable::new();
    let mut vertices = alloc::vec![];
    let mut indices = alloc::vec![];

    let pi = core::f32::consts::PI;
    let two_pi = 2.0 * pi;

    // Generate vertices
    for stack in 0..=stacks {
        let stack_f = stack as f32 / stacks as f32;
        let lat = pi * stack_f; // 0 to π
        let (sin_lat, cos_lat) = trig.sin_cos(lat);

        for slice in 0..=slices {
            let slice_f = slice as f32 / slices as f32;
            let lon = two_pi * slice_f; // 0 to 2π
            let (sin_lon, cos_lon) = trig.sin_cos(lon);

            let x = cos_lon * sin_lat;
            let y = cos_lat;
            let z = sin_lon * sin_lat;

            let u = slice_f;
            let v = stack_f;

            vertices.push(MeshVertex {
                position: Vec3::new(x, y, z),
                normal: Vec3::new(x, y, z), // Already normalized (unit sphere)
                uv: Vec2::new(u, v),
                color: [255, 255, 255, 255],
            });
        }
    }

    // Generate indices (two triangles per quad)
    for stack in 0..stacks {
        for slice in 0..slices {
            let a = (stack * (slices + 1)) + slice;
            let b = a + 1;
            let c = a + slices + 1;
            let d = c + 1;

            if let (Ok(av), Ok(bv), Ok(cv), Ok(dv)) = (
                u16::try_from(a),
                u16::try_from(b),
                u16::try_from(c),
                u16::try_from(d),
            ) {
                // First triangle
                indices.push(av);
                indices.push(cv);
                indices.push(bv);

                // Second triangle
                indices.push(bv);
                indices.push(cv);
                indices.push(dv);
            }
        }
    }

    Mesh::new(vertices, indices)
}
