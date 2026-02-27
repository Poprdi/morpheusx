use morpheus_gfx3d::pipeline::{Pipeline, Material};
use morpheus_gfx3d::math::mat4::Mat4;
use morpheus_gfx3d::math::vec::Vec3;
use morpheus_gfx3d::scene::mesh::Mesh;
use morpheus_gfx3d::light::LightEnv;
use morpheus_gfx3d::target::RenderTarget;

use crate::hud::Framebuf;

const MAX_STARS: usize = 270;
const GALAXY_POS: Vec3 = Vec3 { x: 0.0, y: -15.0, z: -35.0 };

#[derive(Clone, Copy)]
struct Star {
    x: f32,
    y: f32,
    z: f32,
    bright: u8,
    size: u8,
}

impl Star {
    const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0, bright: 0, size: 1 };
}

pub struct Backdrop {
    stars: [Star; MAX_STARS],
    count: usize,
}

pub struct GalaxyAssets {
    pub core: Mesh,
    pub glow: Mesh,
    pub ring1: Mesh,
    pub ring2: Mesh,
    pub ring3: Mesh,
}

impl Backdrop {
    pub fn new() -> Self {
        let mut stars = [Star::ZERO; MAX_STARS];
        let mut rng = 0xDEAD_BEEFu32;
        let mut count = 0usize;

        for _ in 0..40 {
            if count >= MAX_STARS { break; }
            stars[count] = gen_star(&mut rng, 35.0, 55.0, 180, 255, 2);
            count += 1;
        }
        for _ in 0..80 {
            if count >= MAX_STARS { break; }
            stars[count] = gen_star(&mut rng, 70.0, 120.0, 100, 170, 1);
            count += 1;
        }
        for _ in 0..150 {
            if count >= MAX_STARS { break; }
            stars[count] = gen_star(&mut rng, 140.0, 250.0, 40, 90, 1);
            count += 1;
        }

        Self { stars, count }
    }
}

impl GalaxyAssets {
    pub fn new() -> Self {
        Self {
            core: Mesh::sphere(4, 8),
            glow: Mesh::sphere(5, 10),
            ring1: Mesh::torus(3.0, 0.25, 16, 4),
            ring2: Mesh::torus(5.5, 0.15, 20, 4),
            ring3: Mesh::torus(8.0, 0.08, 24, 4),
        }
    }
}

pub fn render_stars(
    fb: &Framebuf,
    backdrop: &Backdrop,
    cam_pos: Vec3,
    cam_yaw: f32,
    cam_pitch: f32,
    fov_y: f32,
    time_ns: u64,
) {
    let sy = fast_sin(cam_yaw);
    let cy = fast_cos(cam_yaw);
    let sp = fast_sin(cam_pitch);
    let cp = fast_cos(cam_pitch);

    let fwd_x = -sy * cp;
    let fwd_y = sp;
    let fwd_z = -cy * cp;

    let right_x = cy;
    let right_z = -sy;

    let up_x = sy * sp;
    let up_y = cp;
    let up_z = cy * sp;

    let focal = (fb.h as f32) * 0.5 / fast_tan(fov_y * 0.5);
    let cx = fb.w as f32 * 0.5;
    let cy_scr = fb.h as f32 * 0.5;

    let twinkle_phase = (time_ns / 50_000_000) as u32;

    for i in 0..backdrop.count {
        let s = &backdrop.stars[i];

        let dx = s.x - cam_pos.x;
        let dy = s.y - cam_pos.y;
        let dz = s.z - cam_pos.z;

        let d_fwd = dx * fwd_x + dy * fwd_y + dz * fwd_z;
        if d_fwd <= 0.5 { continue; }

        let d_right = dx * right_x + dz * right_z;
        let d_up = dx * up_x + dy * up_y + dz * up_z;

        let inv_d = focal / d_fwd;
        let sx = (d_right * inv_d + cx) as i32;
        let sy_px = (-d_up * inv_d + cy_scr) as i32;

        if sx < 0 || sy_px < 0 || sx >= fb.w as i32 || sy_px >= fb.h as i32 { continue; }

        let flicker = {
            let t = (twinkle_phase.wrapping_add(i as u32 * 7919)) % 100;
            0.8 + (t as f32) * 0.004
        };
        let b = ((s.bright as f32 * flicker) as u32).min(255);

        let color = if b > 180 {
            (b << 16) | (b << 8) | (b * 9 / 10)
        } else if b > 100 {
            (b << 16) | (b << 8) | b
        } else {
            ((b * 8 / 10) << 16) | ((b * 9 / 10) << 8) | b
        };

        fb.put(sx as u32, sy_px as u32, color);
        if s.size >= 2 {
            let sx1 = sx as u32 + 1;
            let sy1 = sy_px as u32 + 1;
            if sx1 < fb.w { fb.put(sx1, sy_px as u32, color); }
            if sy1 < fb.h { fb.put(sx as u32, sy1, color); }
            if sx1 < fb.w && sy1 < fb.h { fb.put(sx1, sy1, color); }
        }
    }
}

pub fn render_galaxy<T: RenderTarget>(
    pipeline: &mut Pipeline,
    target: &mut T,
    lights: &LightEnv,
    assets: &GalaxyAssets,
    time_ns: u64,
    load_pct: f32,
) {
    let t_sec = (time_ns as f64 / 1_000_000_000.0) as f32;
    let angle = t_sec * 0.15;
    let sa = fast_sin(angle);
    let ca = fast_cos(angle);
    let rot_y = Mat4::rotation_y(sa, ca);

    let load_t = (load_pct / 100.0).min(1.0);
    let pulse = 1.0 + load_t * 0.3;

    let core_s = 0.5 * pulse;
    let core_model = Mat4::translation(GALAXY_POS.x, GALAXY_POS.y, GALAXY_POS.z)
        .mul(&rot_y)
        .mul(&Mat4::scale(core_s, core_s, core_s));
    let core_b = 0.7 + load_t * 0.3;
    let core_mat = Material::solid(core_b, core_b * 0.9, core_b * 0.6);
    pipeline.draw_mesh(&assets.core, &core_model, &core_mat, lights, target);

    let glow_r = 1.5 * pulse;
    let glow_model = Mat4::translation(GALAXY_POS.x, GALAXY_POS.y, GALAXY_POS.z)
        .mul(&rot_y)
        .mul(&Mat4::scale(glow_r, glow_r, glow_r));
    let glow_mat = Material::solid(0.15, 0.08, 0.25);
    let was_cull = pipeline.backface_cull;
    pipeline.backface_cull = false;
    pipeline.draw_mesh(&assets.glow, &glow_model, &glow_mat, lights, target);
    pipeline.backface_cull = was_cull;

    let tilt1_s = fast_sin(0.15);
    let tilt1_c = fast_cos(0.15);
    let ring1_model = Mat4::translation(GALAXY_POS.x, GALAXY_POS.y, GALAXY_POS.z)
        .mul(&rot_y)
        .mul(&Mat4::rotation_x(tilt1_s, tilt1_c));
    let ring1_mat = Material::solid(0.20, 0.12, 0.45);
    pipeline.draw_mesh(&assets.ring1, &ring1_model, &ring1_mat, lights, target);

    let a2 = angle * 0.7 + 1.0;
    let ring2_rot = Mat4::rotation_y(fast_sin(a2), fast_cos(a2));
    let tilt2_s = fast_sin(0.12);
    let tilt2_c = fast_cos(0.12);
    let ring2_model = Mat4::translation(GALAXY_POS.x, GALAXY_POS.y, GALAXY_POS.z)
        .mul(&ring2_rot)
        .mul(&Mat4::rotation_z(tilt2_s, tilt2_c));
    let ring2_mat = Material::solid(0.12, 0.08, 0.35);
    pipeline.draw_mesh(&assets.ring2, &ring2_model, &ring2_mat, lights, target);

    let a3 = angle * 0.4 + 2.0;
    let ring3_rot = Mat4::rotation_y(fast_sin(a3), fast_cos(a3));
    let tilt3_s = fast_sin(-0.1);
    let tilt3_c = fast_cos(-0.1);
    let ring3_model = Mat4::translation(GALAXY_POS.x, GALAXY_POS.y, GALAXY_POS.z)
        .mul(&ring3_rot)
        .mul(&Mat4::rotation_x(tilt3_s, tilt3_c));
    let ring3_mat = Material::solid(0.06, 0.04, 0.20);
    pipeline.draw_mesh(&assets.ring3, &ring3_model, &ring3_mat, lights, target);
}

fn gen_star(rng: &mut u32, min_dist: f32, max_dist: f32, min_b: u8, max_b: u8, size: u8) -> Star {
    loop {
        let x = rand_f32(rng) * 2.0 - 1.0;
        let y = rand_f32(rng) * 2.0 - 1.0;
        let z = rand_f32(rng) * 2.0 - 1.0;
        let r2 = x * x + y * y + z * z;
        if r2 > 0.01 && r2 < 1.0 {
            let dist = min_dist + rand_f32(rng) * (max_dist - min_dist);
            let r = fast_sqrt(r2);
            let s = dist / r;
            let range = (max_b as u32).saturating_sub(min_b as u32).max(1);
            let b = min_b.saturating_add((xorshift32(rng) % range) as u8);
            return Star { x: x * s, y: y * s, z: z * s, bright: b, size };
        }
    }
}

fn xorshift32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

fn rand_f32(state: &mut u32) -> f32 {
    (xorshift32(state) & 0xFFFF) as f32 / 65535.0
}

fn fast_sin(x: f32) -> f32 {
    let pi = core::f32::consts::PI;
    let mut t = x % (2.0 * pi);
    if t < 0.0 { t += 2.0 * pi; }
    let sign = if t > pi { t -= pi; -1.0 } else { 1.0 };
    let y = t * (pi - t);
    sign * (16.0 * y) / (5.0 * pi * pi - 4.0 * y)
}

fn fast_cos(x: f32) -> f32 {
    fast_sin(x + core::f32::consts::FRAC_PI_2)
}

fn fast_tan(x: f32) -> f32 {
    let c = fast_cos(x);
    if c > -0.0001 && c < 0.0001 { return 1000.0; }
    fast_sin(x) / c
}

fn fast_sqrt(x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }
    let i = f32::to_bits(x);
    let i = (i >> 1) + 0x1FC00000;
    let y = f32::from_bits(i);
    0.5 * (y + x / y)
}
