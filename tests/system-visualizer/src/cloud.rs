use morpheus_gfx3d::pipeline::{Pipeline, Material};
use morpheus_gfx3d::math::mat4::Mat4;
use morpheus_gfx3d::math::vec::Vec3;
use morpheus_gfx3d::scene::mesh::Mesh;
use morpheus_gfx3d::light::LightEnv;
use morpheus_gfx3d::target::RenderTarget;

use crate::state::SystemState;
use crate::layout::ProcessLayout;

pub struct CloudAssets {
    pub sphere_lo: Mesh,
    pub sphere_hi: Mesh,
    pub ring: Mesh,
//    pub connector: Mesh,
}

impl CloudAssets {
    pub fn new() -> Self {
        Self {
            sphere_lo: Mesh::sphere(6, 12),
            sphere_hi: Mesh::sphere(10, 20),
            ring: Mesh::torus(1.0, 0.03, 24, 6),
//            connector: Mesh::cylinder(0.02, 1.0, 4),
        }
    }
}

pub fn render_cloud<T: RenderTarget>(
    pipeline: &mut Pipeline,
    target: &mut T,
    lights: &LightEnv,
    assets: &CloudAssets,
    state: &SystemState,
    layout: &ProcessLayout,
    selected: Option<usize>,
    time_ns: u64,
) {
    let n = layout.count.min(state.proc_count);

    // Parent-child connectors
    for i in 0..n {
        let proc = match state.process(i) {
            Some(p) => p,
            None => continue,
        };
        if let Some(pi) = state.find_index_by_pid(proc.ppid) {
            if pi != i && pi < n {
                let a = layout.smoothed(pi);
                let b = layout.smoothed(i);
                //draw_connector(pipeline, target, lights, assets, a, b);
            }
        }
    }

    // Process spheres
    for i in 0..n {
        let proc = match state.process(i) {
            Some(p) => p,
            None => continue,
        };
        let pos = layout.smoothed(i);
        let radius = layout.radii[i];
        let is_sel = selected == Some(i);

        let (r, g, b) = process_color(proc.state, proc.cpu_pct);
        let scale = Mat4::scale(radius, radius, radius);
        let model = Mat4::translation(pos.x, pos.y, pos.z).mul(&scale);
        let material = Material::solid(r, g, b);

        let mesh = if is_sel || radius > 0.5 { &assets.sphere_hi } else { &assets.sphere_lo };
        pipeline.draw_mesh(mesh, &model, &material, lights, target);

        if is_sel {
            draw_selection_ring(pipeline, target, lights, assets, pos, radius, time_ns);
        }
    }
}


fn draw_selection_ring<T: RenderTarget>(
    pipeline: &mut Pipeline,
    target: &mut T,
    lights: &LightEnv,
    assets: &CloudAssets,
    pos: Vec3,
    radius: f32,
    time_ns: u64,
) {
    let phase = (time_ns % 3_000_000_000) as f32 / 3_000_000_000.0;
    let pulse = 1.0 + 0.15 * fast_sin(phase * 6.2832);
    let ring_radius = radius * 1.3 * pulse;

    let model = Mat4::translation(pos.x, pos.y, pos.z)
        .mul(&Mat4::scale(ring_radius, radius, ring_radius));
    let material = Material::solid(0.2, 1.0, 0.5);
    pipeline.draw_mesh(&assets.ring, &model, &material, lights, target);
}


fn process_color(state: u32, cpu_pct: f32) -> (f32, f32, f32) {
    match state {
        1 => {
            let intensity = cpu_pct / 100.0;
            match intensity {
                i if i < 0.25 => (0.1, 0.3, 0.1),        // dark green
                i if i < 0.5 => (0.2, 0.6, 0.2),         // light green
                i if i < 0.65 => (0.6, 0.8, 0.1),        // yellow-green
                i if i < 0.8 => (0.9, 0.7, 0.1),         // yellow-orange
                i if i < 0.9 => (0.95, 0.5, 0.1),        // orange
                _ => (0.8, 0.1, 0.1),                     // dark red
            }
        }
        0 => (0.3, 0.2, 0.1),      // idle: dark brown
        2 => (0.2, 0.4, 0.8),      // sleeping: blue
        3 => (0.4, 0.4, 0.4),      // stopped: gray
        4 => (0.2, 0.2, 0.2),      // zombie: dark gray
        _ => (0.5, 0.5, 0.5),      // unknown: neutral gray
    }
}

fn fast_sin(x: f32) -> f32 {
    let pi = core::f32::consts::PI;
    let mut t = x % (2.0 * pi);
    if t < 0.0 { t += 2.0 * pi; }
    let sign = if t > pi { t -= pi; -1.0 } else { 1.0 };
    let y = t * (pi - t);
    sign * (16.0 * y) / (5.0 * pi * pi - 4.0 * y)
}

fn fast_inv_sqrt(x: f32) -> f32 {
    let half = 0.5 * x;
    let i = f32::to_bits(x);
    let i = 0x5f3759df - (i >> 1);
    let y = f32::from_bits(i);
    y * (1.5 - half * y * y)
}
