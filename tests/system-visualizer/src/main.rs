#![no_std]
#![no_main]
extern crate alloc;
use alloc::boxed::Box;

mod backdrop;
mod cloud;
mod font;
mod hud;
mod input;
mod layout;
mod state;

use libmorpheus::entry;
use libmorpheus::hw::{fb_blit, fb_info, fb_lock, fb_map};
use libmorpheus::process;
use libmorpheus::time;
use morpheus_gfx3d::camera::Camera;
use morpheus_gfx3d::light::{DirLight, LightEnv};
use morpheus_gfx3d::math::vec::Vec3;
use morpheus_gfx3d::pipeline::Pipeline;
use morpheus_gfx3d::target::{DirectTarget, TargetPixelFormat};

use hud::Framebuf;
use input::{Action, InputState};
use layout::ProcessLayout;
use state::SystemState;

entry!(main);

struct OrbitCam {
    yaw: f32,
    pitch: f32,
    dist: f32,
    focus: Vec3,
    yaw_vel: f32,
    pitch_vel: f32,
    dist_vel: f32,
}

impl OrbitCam {
    fn new() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.35,
            dist: 16.0,
            focus: Vec3::new(0.0, -1.0, 0.0),
            yaw_vel: 0.0,
            pitch_vel: 0.0,
            dist_vel: 0.0,
        }
    }

    fn update(&mut self, dt: f32) {
        let decay = clamp(1.0 - fast_exp(-8.0 * dt), 0.0, 1.0);
        self.yaw += self.yaw_vel * dt;
        self.pitch += self.pitch_vel * dt;
        self.dist += self.dist_vel * dt;

        self.yaw_vel *= 1.0 - decay;
        self.pitch_vel *= 1.0 - decay;
        self.dist_vel *= 1.0 - decay;

        let two_pi = 2.0 * core::f32::consts::PI;
        while self.yaw < 0.0 {
            self.yaw += two_pi;
        }
        while self.yaw >= two_pi {
            self.yaw -= two_pi;
        }
        self.pitch = clamp(self.pitch, -1.4, 1.4);
        self.dist = clamp(self.dist, 3.0, 60.0);
    }

    fn apply(&self, camera: &mut Camera) {
        let cp = fast_cos(self.pitch);
        let sp = fast_sin(self.pitch);
        let cy = fast_cos(self.yaw);
        let sy = fast_sin(self.yaw);

        camera.position = Vec3::new(
            self.focus.x + self.dist * cp * sy,
            self.focus.y + self.dist * sp,
            self.focus.z + self.dist * cp * cy,
        );

        let dx = self.focus.x - camera.position.x;
        let dy = self.focus.y - camera.position.y;
        let dz = self.focus.z - camera.position.z;
        let xz_len = fast_sqrt(dx * dx + dz * dz).max(0.0001);

        camera.yaw = fast_atan2(-dx, -dz);
        camera.pitch = fast_atan2(dy, xz_len);
    }
}

fn main() -> i32 {
    let info = match fb_info() {
        Ok(i) => i,
        Err(_) => return 1,
    };

    let fb_w = info.width;
    let fb_h = info.height;
    let fb_stride = info.stride / 4;
    let fb_format = match info.format {
        0 => TargetPixelFormat::Rgbx,
        1 => TargetPixelFormat::Bgrx,
        _ => TargetPixelFormat::Bgrx,
    };

    let fb_vaddr = match fb_map() {
        Ok(a) => a,
        Err(_) => return 1,
    };

    if fb_lock().is_err() {
        return 1;
    }

    let cloud_assets = Box::new(cloud::CloudAssets::new());
    let backdrop_stars = Box::new(backdrop::Backdrop::new());
    let galaxy_assets = Box::new(backdrop::GalaxyAssets::new());

    let mut target =
        unsafe { DirectTarget::new(fb_vaddr as *mut u32, fb_w, fb_h, fb_stride, fb_format) };

    let mut pipeline = Box::new(Pipeline::new(fb_w, fb_h));
    pipeline.backface_cull = true;
    pipeline.wireframe = false;
    pipeline.sample_mode = morpheus_gfx3d::texture::sample::SampleMode::Nearest;

    let mut lights = LightEnv::new();
    lights.ambient = [0.18, 0.20, 0.22];
    lights.dir_lights.push(DirLight {
        direction: Vec3::new(0.4, 0.9, -0.3).normalize(),
        color: [0.85, 0.90, 1.0],
    });
    lights.dir_lights.push(DirLight {
        direction: Vec3::new(-0.6, 0.2, 0.7).normalize(),
        color: [0.25, 0.15, 0.35],
    });

    let aspect = fb_w as f32 / fb_h.max(1) as f32;
    let mut camera = Camera::new(aspect);
    camera.fov_y = 0.9;

    let fb = Framebuf {
        ptr: fb_vaddr as *mut u32,
        w: fb_w,
        h: fb_h,
        stride: fb_stride,
    };

    let mut sys_state = Box::new(SystemState::new());
    let mut input = InputState::new();
    let mut proc_layout = Box::new(ProcessLayout::new());
    let mut orbit = OrbitCam::new();

    let mut selected: Option<usize> = None;
    let mut paused = false;
    let mut show_hud = true;
    let mut slow_motion = false;
    let mut pinned = false;

    let mut fps_count = 0u32;
    let mut fps_window_start = time::clock_gettime();
    let mut fps_display = 0u32;
    let mut prev_frame_ns = time::clock_gettime();

    let orbit_accel = 6.0f32; // base rotation acceleration (scaled by speed_mult)
    let zoom_accel = 12.0f32;
    let mouse_sensitivity = 0.004f32;
    let mut speed_mult = 1.0f32; // adjustable via [ and ]

    sys_state.poll();

    loop {
        let now = time::clock_gettime();
        let raw_dt_ns = now.saturating_sub(prev_frame_ns).max(1);
        let dt = if slow_motion {
            (raw_dt_ns as f32) / 16_000_000_000.0
        } else {
            (raw_dt_ns as f32) / 1_000_000_000.0
        };
        prev_frame_ns = now;

        input.poll();

        if input.has(Action::Quit) {
            break;
        }
        if input.has(Action::TogglePause) {
            paused = !paused;
        }
        if input.has(Action::ToggleHud) {
            show_hud = !show_hud;
        }
        if input.has(Action::ToggleSlow) {
            slow_motion = !slow_motion;
        }
        if input.has(Action::ResetView) {
            orbit.focus = Vec3::new(0.0, -1.0, 0.0);
            orbit.dist = 16.0;
            orbit.yaw = 0.0;
            orbit.pitch = 0.35;
            orbit.yaw_vel = 0.0;
            orbit.pitch_vel = 0.0;
            orbit.dist_vel = 0.0;
            pinned = false;
        }
        if input.has(Action::TogglePin) {
            pinned = !pinned;
        }
        if input.has(Action::SpeedUp) {
            speed_mult = (speed_mult + 0.25).min(3.0);
        }
        if input.has(Action::SpeedDown) {
            speed_mult = (speed_mult - 0.25).max(0.25);
        }

        if (input.held & input::HELD_A) != 0 {
            orbit.yaw_vel -= orbit_accel * speed_mult * dt;
        }
        if (input.held & input::HELD_D) != 0 {
            orbit.yaw_vel += orbit_accel * speed_mult * dt;
        }
        if (input.held & input::HELD_W) != 0 {
            orbit.pitch_vel += orbit_accel * speed_mult * dt * 0.7;
        }
        if (input.held & input::HELD_S) != 0 {
            orbit.pitch_vel -= orbit_accel * speed_mult * dt * 0.7;
        }
        if (input.held & input::HELD_Z) != 0 {
            orbit.dist_vel -= zoom_accel * speed_mult * dt;
        }
        if (input.held & input::HELD_X) != 0 {
            orbit.dist_vel += zoom_accel * speed_mult * dt;
        }

        if input.mouse_left {
            orbit.yaw_vel += input.mouse_dx * mouse_sensitivity;
            orbit.pitch_vel -= input.mouse_dy * mouse_sensitivity;
        }
        if input.mouse_right {
            orbit.dist_vel -= input.mouse_dy * 0.1;
        }

        orbit.update(dt);
        orbit.apply(&mut camera);

        if input.has(Action::SelectNext) {
            selected = Some(match selected {
                Some(s) if s + 1 < sys_state.proc_count => s + 1,
                _ => 0,
            });
        }
        if input.has(Action::SelectPrev) {
            selected = Some(match selected {
                Some(s) if s > 0 => s - 1,
                _ => sys_state.proc_count.saturating_sub(1),
            });
        }

        if input.has(Action::KillSelected) {
            if let Some(idx) = selected {
                if let Some(proc) = sys_state.process(idx) {
                    let _ = process::kill(proc.pid, process::signal::SIGKILL);
                }
            }
        }

        let digit_actions = [
            (Action::SelectDigit1, 1u32),
            (Action::SelectDigit2, 2),
            (Action::SelectDigit3, 3),
            (Action::SelectDigit4, 4),
            (Action::SelectDigit5, 5),
            (Action::SelectDigit6, 6),
            (Action::SelectDigit7, 7),
            (Action::SelectDigit8, 8),
            (Action::SelectDigit9, 9),
        ];
        for &(act, digit) in &digit_actions {
            if input.has(act) {
                if let Some(idx) = sys_state.find_index_by_pid(digit) {
                    selected = Some(idx);
                } else {
                    let d = digit as usize;
                    if d > 0 && d <= sys_state.proc_count {
                        selected = Some(d - 1);
                    }
                }
            }
        }

        if input.has(Action::Focus) {
            if let Some(idx) = selected {
                let pos = proc_layout.smoothed(idx);
                orbit.focus = pos;
                orbit.dist = 5.0;
                pinned = true;
            }
        }

        if input.has(Action::Unfocus) {
            orbit.focus = Vec3::new(0.0, -1.0, 0.0);
            orbit.dist = 16.0;
            orbit.yaw = 0.0;
            orbit.pitch = 0.35;
            orbit.yaw_vel = 0.0;
            orbit.pitch_vel = 0.0;
            orbit.dist_vel = 0.0;
            pinned = false;
        }

        if pinned {
            if let Some(idx) = selected {
                orbit.focus = proc_layout.smoothed(idx);
            }
        }

        if !paused && sys_state.should_poll(now) {
            sys_state.poll();
        }

        if !paused {
            proc_layout.update(&sys_state, dt);
        }

        target.clear(0x00060A0E);
        pipeline.begin_frame();
        pipeline.set_camera(&camera);

        // Backdrop: stars (2D) + galaxy (3D) — rendered behind process cloud
        backdrop::render_stars(
            &fb,
            &backdrop_stars,
            camera.position,
            camera.yaw,
            camera.pitch,
            camera.fov_y,
            now,
        );
        backdrop::render_galaxy(
            &mut pipeline,
            &mut target,
            &lights,
            &galaxy_assets,
            now,
            sys_state.total_cpu_pct,
        );

        // Clear depth after galaxy so its torus rings can never occlude process
        // spheres — galaxy is a pure background element regardless of camera angle.
        target.clear_depth();

        cloud::render_cloud(
            &mut pipeline,
            &mut target,
            &lights,
            &cloud_assets,
            &sys_state,
            &proc_layout,
            selected,
            now,
        );

        if show_hud {
            hud::draw_system_panel(&fb, &sys_state);
            hud::draw_process_panel(&fb, &sys_state, selected);
            hud::draw_state_bar(&fb, &sys_state);
            hud::draw_load_graph(&fb, &sys_state);
            hud::draw_controls(&fb);

            if let Some(idx) = selected {
                if let Some(proc) = sys_state.process(idx) {
                    hud::draw_selected_detail(&fb, proc);
                }
            }
        }

        hud::draw_status_flags(&fb, paused, slow_motion, pinned);

        fps_count = fps_count.saturating_add(1);
        let fps_elapsed = now.saturating_sub(fps_window_start);
        if fps_elapsed >= 1_000_000_000 {
            fps_display = (fps_count as u64 * 1_000_000_000 / fps_elapsed) as u32;
            fps_count = 0;
            fps_window_start = now;
        }

        let latency_ms = (raw_dt_ns / 1_000_000).min(999) as u32;
        hud::draw_fps(&fb, fps_display, latency_ms, speed_mult);

        let _ = fb_blit();
    }

    0
}

fn fast_sin(x: f32) -> f32 {
    let pi = core::f32::consts::PI;
    let mut t = x % (2.0 * pi);
    if t < 0.0 {
        t += 2.0 * pi;
    }
    let sign = if t > pi {
        t -= pi;
        -1.0
    } else {
        1.0
    };
    let y = t * (pi - t);
    sign * (16.0 * y) / (5.0 * pi * pi - 4.0 * y)
}

fn fast_cos(x: f32) -> f32 {
    fast_sin(x + core::f32::consts::FRAC_PI_2)
}

fn fast_sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let i = f32::to_bits(x);
    let i = (i >> 1) + 0x1FC00000;
    let y = f32::from_bits(i);
    0.5 * (y + x / y)
}

fn fast_atan2(y: f32, x: f32) -> f32 {
    let pi = core::f32::consts::PI;
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let ax = if x < 0.0 { -x } else { x };
    let ay = if y < 0.0 { -y } else { y };
    let (mn, mx) = if ax < ay { (ax, ay) } else { (ay, ax) };
    let a = mn / mx;
    let s = a * a;
    let r = ((-0.0464964749 * s + 0.15931422) * s - 0.327622764) * s * a + a;
    let r = if ax < ay { 1.5707963 - r } else { r };
    let r = if x < 0.0 { pi - r } else { r };
    if y < 0.0 {
        -r
    } else {
        r
    }
}

fn fast_exp(x: f32) -> f32 {
    if x > 20.0 {
        return f32::MAX;
    }
    if x < -20.0 {
        return 0.0;
    }
    let t = 1.0 + x / 256.0;
    let mut r = t;
    for _ in 0..8 {
        r = r * r;
    }
    r
}

fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}
