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
    pub connector: Mesh,
    pub grid: Mesh,
}

impl CloudAssets {
    pub fn new() -> Self {
        Self {
            sphere_lo: Mesh::sphere(6, 12),
            sphere_hi: Mesh::sphere(10, 20),
            ring: Mesh::torus(1.0, 0.03, 24, 6),
            connector: Mesh::cylinder(0.02, 1.0, 4),
            grid: Mesh::plane(30.0, 30.0, 30, 30),
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

    // Grid floor
    {
        let model = Mat4::translation(0.0, -4.0, 0.0);
        let material = Material::solid(0.08, 0.12, 0.10);
        pipeline.wireframe = true;
        pipeline.draw_mesh(&assets.grid, &model, &material, lights, target);
        pipeline.wireframe = false;
    }

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
                draw_connector(pipeline, target, lights, assets, a, b);
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

fn draw_connector<T: RenderTarget>(
    pipeline: &mut Pipeline,
    target: &mut T,
    lights: &LightEnv,
    assets: &CloudAssets,
    from: Vec3,
    to: Vec3,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let dz = to.z - from.z;
    let len_sq = dx * dx + dy * dy + dz * dz;
    if len_sq < 0.01 { return; }

    let len = len_sq * fast_inv_sqrt(len_sq);
    let mid = Vec3::new(
        (from.x + to.x) * 0.5,
        (from.y + to.y) * 0.5,
        (from.z + to.z) * 0.5,
    );

    let model = Mat4::translation(mid.x, mid.y, mid.z)
        .mul(&Mat4::scale(0.5, len, 0.5));
    let material = Material::solid(0.12, 0.22, 0.18);
    pipeline.draw_mesh(&assets.connector, &model, &material, lights, target);
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
    let r = radius * 1.3 * pulse;

    let model = Mat4::translation(pos.x, pos.y, pos.z)
        .mul(&Mat4::scale(r, r, r));
    let material = Material::solid(0.2, 1.0, 0.5);
    pipeline.draw_mesh(&assets.ring, &model, &material, lights, target);
}

fn process_color(state: u32, cpu_pct: f32) -> (f32, f32, f32) {
    match state {
        1 => {
            let intensity = 0.4 + (cpu_pct / 100.0) * 0.6;
            (0.15, if intensity > 1.0 { 1.0 } else { intensity }, 0.15)
        }
        0 => (0.7, 0.7, 0.2),
        2 => (0.2, 0.4, 0.8),
        3 => (0.4, 0.4, 0.4),
        4 => (0.2, 0.2, 0.2),
        _ => (0.5, 0.5, 0.5),
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
