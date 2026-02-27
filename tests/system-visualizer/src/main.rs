#![no_std]
#![no_main]
extern crate alloc;

mod font;
mod state;
mod input;
mod hud;
mod layout;
mod cloud;

use libmorpheus::entry;
use libmorpheus::hw::{fb_info, fb_lock, fb_map, fb_blit};
use libmorpheus::process;
use libmorpheus::time;
use morpheus_gfx3d::pipeline::Pipeline;
use morpheus_gfx3d::math::vec::Vec3;
use morpheus_gfx3d::light::{LightEnv, DirLight};
use morpheus_gfx3d::camera::Camera;
use morpheus_gfx3d::target::{TargetPixelFormat, DirectTarget};

use state::SystemState;
use input::{InputState, Action};
use hud::Framebuf;
use layout::ProcessLayout;

entry!(main);

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

    let cloud_assets = cloud::CloudAssets::new();

    let mut target = unsafe {
        DirectTarget::new(fb_vaddr as *mut u32, fb_w, fb_h, fb_stride, fb_format)
    };

    let mut pipeline = Pipeline::new(fb_w, fb_h);
    pipeline.backface_cull = true;
    pipeline.wireframe = false;
    pipeline.sample_mode = morpheus_gfx3d::texture::sample::SampleMode::Nearest;

    let mut lights = LightEnv::new();
    lights.ambient = [0.25, 0.28, 0.25];
    lights.dir_lights.push(DirLight {
        direction: Vec3::new(0.4, 0.8, -0.4).normalize(),
        color: [0.7, 0.75, 0.8],
    });
    lights.dir_lights.push(DirLight {
        direction: Vec3::new(-0.5, 0.3, 0.7).normalize(),
        color: [0.2, 0.25, 0.2],
    });

    let aspect = fb_w as f32 / fb_h.max(1) as f32;
    let mut camera = Camera::new(aspect);
    camera.position = Vec3::new(0.0, 2.0, 14.0);
    camera.yaw = 0.0;
    camera.pitch = -0.15;
    camera.fov_y = 0.9;

    let fb = Framebuf {
        ptr: fb_vaddr as *mut u32,
        w: fb_w,
        h: fb_h,
        stride: fb_stride,
    };

    let mut sys_state = SystemState::new();
    let mut input = InputState::new();
    let mut proc_layout = ProcessLayout::new();

    let mut selected: Option<usize> = None;
    let mut paused = false;
    let mut show_hud = true;

    let mut fps_count = 0u32;
    let mut fps_window_start = time::clock_gettime();
    let mut fps_display = 0u32;
    let mut prev_frame_ns = time::clock_gettime();

    let rotate_speed = 0.04f32;
    let zoom_speed = 0.5f32;
    let mouse_sensitivity = 0.003f32;

    sys_state.poll();

    loop {
        let now = time::clock_gettime();
        let dt_ns = now.saturating_sub(prev_frame_ns).max(1);
        let dt = (dt_ns as f32) / 1_000_000_000.0;
        prev_frame_ns = now;

        // Input
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

        // Camera
        for &action in input.iter_actions() {
            match action {
                Action::RotateLeft => camera.yaw -= rotate_speed,
                Action::RotateRight => camera.yaw += rotate_speed,
                Action::RotateUp => camera.pitch += rotate_speed,
                Action::RotateDown => camera.pitch -= rotate_speed,
                Action::ZoomIn => {
                    let trig = pipeline.trig();
                    camera.translate(zoom_speed, 0.0, 0.0, trig);
                }
                Action::ZoomOut => {
                    let trig = pipeline.trig();
                    camera.translate(-zoom_speed, 0.0, 0.0, trig);
                }
                _ => {}
            }
        }

        // Mouse look
        if input.mouse_left {
            camera.yaw += input.mouse_dx * mouse_sensitivity;
            camera.pitch -= input.mouse_dy * mouse_sensitivity;
        }

        // Clamp pitch
        let max_pitch = 1.5;
        if camera.pitch > max_pitch { camera.pitch = max_pitch; }
        if camera.pitch < -max_pitch { camera.pitch = -max_pitch; }

        // Selection
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

        if input.has(Action::Focus) {
            if let Some(idx) = selected {
                let pos = proc_layout.smoothed(idx);
                camera.position = Vec3::new(pos.x, pos.y + 1.5, pos.z + 4.0);
            }
        }

        if input.has(Action::Unfocus) {
            camera.position = Vec3::new(0.0, 2.0, 14.0);
            camera.yaw = 0.0;
            camera.pitch = -0.15;
        }

        // System state
        if !paused && sys_state.should_poll(now) {
            sys_state.poll();
        }

        // Layout
        if !paused {
            proc_layout.update(&sys_state, dt);
        }

        // Render 3D
        target.clear(0x00080C10);
        pipeline.begin_frame();
        pipeline.set_camera(&camera);

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

        // Render 2D HUD
        if show_hud {
            hud::draw_system_panel(&fb, &sys_state);
            hud::draw_process_panel(&fb, &sys_state, selected);
            hud::draw_load_graph(&fb, &sys_state);
            hud::draw_controls(&fb);

            if let Some(idx) = selected {
                if let Some(proc) = sys_state.process(idx) {
                    hud::draw_selected_detail(&fb, proc);
                }
            }
        }

        // FPS
        fps_count = fps_count.saturating_add(1);
        let fps_elapsed = now.saturating_sub(fps_window_start);
        if fps_elapsed >= 1_000_000_000 {
            fps_display = (fps_count as u64 * 1_000_000_000 / fps_elapsed) as u32;
            fps_count = 0;
            fps_window_start = now;
        }

        let latency_ms = (dt_ns / 1_000_000).min(999) as u32;
        hud::draw_fps(&fb, fps_display, latency_ms);

        let _ = fb_blit();
    }

    0
}
