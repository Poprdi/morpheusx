#![no_std]
#![no_main]
extern crate alloc;

use libmorpheus::entry;
use libmorpheus::hw::{fb_blit, fb_info, fb_lock, fb_map};
use libmorpheus::time;
use morpheus_gfx3d::camera::Camera;
use morpheus_gfx3d::light::{DirLight, LightEnv};
use morpheus_gfx3d::math::mat4::Mat4;
use morpheus_gfx3d::math::trig::TrigTable;
use morpheus_gfx3d::math::vec::Vec3;
use morpheus_gfx3d::pipeline::Material;
use morpheus_gfx3d::pipeline::Pipeline;
use morpheus_gfx3d::scene::mesh::Mesh;
use morpheus_gfx3d::target::{DirectTarget, TargetPixelFormat};

entry!(main);

fn main() -> i32 {
    // ── Get framebuffer info ──
    let fb_info = match fb_info() {
        Ok(info) => info,
        Err(_) => return 1,
    };

    let fb_width = fb_info.width;
    let fb_height = fb_info.height;
    let fb_stride = fb_info.stride / 4;
    let fb_format = match fb_info.format {
        0 => TargetPixelFormat::Rgbx,
        1 => TargetPixelFormat::Bgrx,
        _ => TargetPixelFormat::Bgrx,
    };

    // ── Map framebuffer virtual address ──
    let fb_vaddr = match fb_map() {
        Ok(addr) => addr,
        Err(_) => return 1,
    };

    // ── Take exclusive framebuffer ownership ──
    if fb_lock().is_err() {
        return 1;
    }

    // ── Clear framebuffer to black ──
    clear_framebuffer(fb_vaddr, fb_width, fb_height, fb_stride, 0x00000000);

    // ── Create render target backed directly by the mapped back buffer ──
    // No intermediate copy — the 3D pipeline writes pixels straight into
    // the kernel-visible back buffer.  fb_present() then diffs against
    // the shadow and pushes only changed spans to VRAM.
    let mut target = unsafe {
        DirectTarget::new(
            fb_vaddr as *mut u32,
            fb_width,
            fb_height,
            fb_stride,
            fb_format,
        )
    };

    // ── Initialize 3D pipeline ──
    let mut pipeline = Pipeline::new(fb_width, fb_height);
    pipeline.fog = morpheus_gfx3d::light::FogMode::None;
    pipeline.wireframe = false;
    pipeline.backface_cull = true;
    pipeline.sample_mode = morpheus_gfx3d::texture::sample::SampleMode::Nearest;

    // ── Set up lighting ──
    let mut lights = LightEnv::new();
    lights.ambient = [0.42, 0.42, 0.42];
    lights.dir_lights.push(DirLight {
        direction: Vec3::new(0.4, 0.85, -0.35).normalize(),
        color: [0.85, 0.90, 1.00],
    });
    lights.dir_lights.push(DirLight {
        direction: Vec3::new(-0.6, 0.45, 0.65).normalize(),
        color: [0.25, 0.25, 0.25],
    });

    // ── Create meshes ──
    let cube = Mesh::cube();
    let sphere = Mesh::sphere(12, 24);
    let torus = Mesh::torus(0.6, 0.3, 16, 16);
    let pyramid = Mesh::pyramid();
    let plane = Mesh::plane(2.0, 2.0, 4, 4);
    let cylinder = Mesh::cylinder(0.5, 1.5, 16);
    let lattice_wall = Mesh::plane(28.0, 18.0, 28, 18);
    let lattice_floor = Mesh::plane(28.0, 18.0, 28, 18);

    // ── Set up camera (looking at the center where all meshes are) ──
    let aspect = fb_width as f32 / fb_height.max(1) as f32;
    let mut camera = Camera::new(aspect);
    camera.position = Vec3::new(0.0, 0.8, 10.5);
    camera.yaw = 0.0;
    camera.pitch = 0.0;
    camera.fov_y = 0.87266; // 50° for less perspective exaggeration

    // ── Trig table for rotation ──
    let trig = TrigTable::new();

    // ── Frame statistics ──
    let mut fps_frame_count = 0u32;
    let mut fps_window_start_ns = time::clock_gettime();
    let mut fps_display = 0u32;

    // ── Render loop (spin all shapes forever, until Ctrl+C) ──
    loop {
        let frame_start_ns = time::clock_gettime();

        // Update target state (reset for new frame)
        target.clear(0x00000000); // Black background
        pipeline.begin_frame();
        pipeline.set_camera(&camera);

        // Compute time-based rotation phases
        let nanos = frame_start_ns;
        let phase_y = (nanos % 4_000_000_000) as f32 / 4_000_000_000.0;
        let angle_y = phase_y * 6.2831855;
        let angle_x = angle_y * 0.6;
        let angle_z = angle_y * 0.3;

        let (sin_y, cos_y) = trig.sin_cos(angle_y);
        let (sin_x, cos_x) = trig.sin_cos(angle_x);
        let (sin_z, cos_z) = trig.sin_cos(angle_z);

        let rot_y = Mat4::rotation_y(sin_y, cos_y);
        let rot_x = Mat4::rotation_x(sin_x, cos_x);
        let rot_z = Mat4::rotation_z(sin_z, cos_z);

        // Draw background lattice (wireframe wall + floor)
        {
            pipeline.wireframe = true;
            let wall_model = Mat4::translation(0.0, 0.5, -8.5).mul(&Mat4::rotation_x(1.0, 0.0));
            let floor_model = Mat4::translation(0.0, -3.6, -1.0).mul(&Mat4::scale(1.0, 1.0, 1.0));

            let wall_material = Material::solid(0.16, 0.24, 0.16);
            let floor_material = Material::solid(0.14, 0.20, 0.22);

            pipeline.draw_mesh(
                &lattice_wall,
                &wall_model,
                &wall_material,
                &lights,
                &mut target,
            );
            pipeline.draw_mesh(
                &lattice_floor,
                &floor_model,
                &floor_material,
                &lights,
                &mut target,
            );
            pipeline.wireframe = false;
        }

        // Draw cube (top-left) — rigid single-axis rotation for diagnostics
        {
            let cube_phase = (nanos % 6_000_000_000) as f32 / 6_000_000_000.0;
            let cube_angle = cube_phase * 6.2831855;
            let (cube_sin, cube_cos) = trig.sin_cos(cube_angle);
            let cube_rot = Mat4::rotation_y(cube_sin, cube_cos);

            let model = Mat4::translation(-3.8, 2.2, 0.8)
                .mul(&cube_rot.mul(&Mat4::scale(0.95, 0.95, 0.95)));
            let material = Material::solid(0.3, 0.8, 0.3);
            pipeline.draw_mesh(&cube, &model, &material, &lights, &mut target);
        }

        // Draw sphere (top-center)
        {
            let model = Mat4::translation(0.0, 2.2, 0.0).mul(
                &rot_y
                    .mul(&rot_x)
                    .mul(&rot_z)
                    .mul(&Mat4::scale(0.90, 0.90, 0.90)),
            );
            let material = Material::solid(0.3, 0.3, 0.8);
            pipeline.draw_mesh(&sphere, &model, &material, &lights, &mut target);
        }

        // Draw torus (top-right)
        {
            let model = Mat4::translation(3.8, 2.2, -0.8).mul(
                &rot_y
                    .mul(&rot_x)
                    .mul(&rot_z)
                    .mul(&Mat4::scale(0.95, 0.95, 0.95)),
            );
            let material = Material::solid(0.8, 0.3, 0.3);
            pipeline.draw_mesh(&torus, &model, &material, &lights, &mut target);
        }

        // Draw pyramid (bottom-left)
        {
            let model = Mat4::translation(-3.8, -2.2, -0.4).mul(
                &rot_y
                    .mul(&rot_x)
                    .mul(&rot_z)
                    .mul(&Mat4::scale(1.00, 1.00, 1.00)),
            );
            let material = Material::solid(0.8, 0.8, 0.3);
            pipeline.draw_mesh(&pyramid, &model, &material, &lights, &mut target);
        }

        // Draw plane (bottom-center)
        {
            let phase_plane = (nanos % 2_000_000_000) as f32 / 2_000_000_000.0;
            let angle_plane = phase_plane * 6.2831855;
            let (sin_p, cos_p) = trig.sin_cos(angle_plane);
            let model = Mat4::translation(0.0, -2.2, 0.9)
                .mul(&Mat4::rotation_x(sin_p, cos_p).mul(&Mat4::scale(0.95, 0.95, 0.95)));
            let material = Material::solid(0.3, 0.8, 0.8);
            pipeline.draw_mesh(&plane, &model, &material, &lights, &mut target);
        }

        // Draw cylinder (bottom-right)
        {
            let phase_cyl = (nanos % 4_000_000_000) as f32 / 4_000_000_000.0;
            let angle_cyl = phase_cyl * 6.2831855;
            let (sin_c, cos_c) = trig.sin_cos(angle_cyl);
            let model = Mat4::translation(3.8, -2.2, 0.3)
                .mul(&Mat4::rotation_z(sin_c, cos_c).mul(&Mat4::scale(0.80, 0.80, 0.80)));
            let material = Material::solid(0.8, 0.3, 0.8);
            pipeline.draw_mesh(&cylinder, &model, &material, &lights, &mut target);
        }

        // Update and draw HUD stats
        let frame_end_ns = time::clock_gettime();
        let frame_ns = frame_end_ns.saturating_sub(frame_start_ns);
        let latency_ms = (frame_ns / 1_000_000).min(999) as u32;

        fps_frame_count = fps_frame_count.saturating_add(1);
        let fps_window_ns = frame_end_ns.saturating_sub(fps_window_start_ns);
        if fps_window_ns >= 1_000_000_000 {
            fps_display =
                ((fps_frame_count as u64).saturating_mul(1_000_000_000) / fps_window_ns) as u32;
            fps_frame_count = 0;
            fps_window_start_ns = frame_end_ns;
        }

        draw_hud(
            fb_vaddr,
            fb_width,
            fb_height,
            fb_stride,
            fps_display,
            latency_ms,
            pipeline.stats.triangles_drawn,
            pipeline.stats.pixels_written,
        );

        // Push completed frame to VRAM (full memcpy — faster than delta for 3D)
        let _ = fb_blit();
    }

    #[allow(unreachable_code)]
    {
        0
    }
}

/// Clear the hardware framebuffer to a solid color.
fn clear_framebuffer(fb_vaddr: u64, fb_width: u32, fb_height: u32, fb_stride: u32, color: u32) {
    let fb_ptr = fb_vaddr as *mut u32;
    for y in 0..fb_height {
        let dst_row = y as usize * fb_stride as usize;
        unsafe {
            for x in 0..fb_width {
                let dst_idx = dst_row + x as usize;
                *fb_ptr.add(dst_idx) = color;
            }
        }
    }
}

fn draw_hud(
    fb_vaddr: u64,
    fb_width: u32,
    fb_height: u32,
    fb_stride: u32,
    fps: u32,
    latency_ms: u32,
    tris: u32,
    pixels: u32,
) {
    let panel_x = 8u32;
    let panel_y = 8u32;
    let panel_w = 176u32;
    let panel_h = 44u32;

    fill_rect(
        fb_vaddr, fb_width, fb_height, fb_stride, panel_x, panel_y, panel_w, panel_h, 0x00101010,
    );

    let fg = 0x00E0FFE0;
    draw_text_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        panel_x + 6,
        panel_y + 6,
        "FPS:",
        fg,
    );
    draw_u32_3(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        panel_x + 36,
        panel_y + 6,
        fps,
        fg,
    );

    draw_text_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        panel_x + 72,
        panel_y + 6,
        "LAT:",
        fg,
    );
    draw_u32_3(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        panel_x + 102,
        panel_y + 6,
        latency_ms,
        fg,
    );
    draw_text_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        panel_x + 122,
        panel_y + 6,
        "MS",
        fg,
    );

    draw_text_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        panel_x + 6,
        panel_y + 24,
        "TRI:",
        fg,
    );
    draw_u32_4(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        panel_x + 36,
        panel_y + 24,
        tris,
        fg,
    );

    draw_text_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        panel_x + 72,
        panel_y + 24,
        "PIX:",
        fg,
    );
    draw_u32_4(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        panel_x + 102,
        panel_y + 24,
        pixels / 100,
        fg,
    );
}

fn fill_rect(
    fb_vaddr: u64,
    fb_width: u32,
    fb_height: u32,
    fb_stride: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: u32,
) {
    let x_end = x.saturating_add(w).min(fb_width);
    let y_end = y.saturating_add(h).min(fb_height);
    let fb_ptr = fb_vaddr as *mut u32;

    for py in y..y_end {
        let row = py as usize * fb_stride as usize;
        for px in x..x_end {
            let idx = row + px as usize;
            unsafe {
                *fb_ptr.add(idx) = color;
            }
        }
    }
}

fn draw_text_5x7(
    fb_vaddr: u64,
    fb_width: u32,
    fb_height: u32,
    fb_stride: u32,
    x: u32,
    y: u32,
    text: &str,
    color: u32,
) {
    let mut cx = x;
    for ch in text.bytes() {
        draw_glyph_5x7(fb_vaddr, fb_width, fb_height, fb_stride, cx, y, ch, color);
        cx = cx.saturating_add(6);
    }
}

fn draw_u32_3(
    fb_vaddr: u64,
    fb_width: u32,
    fb_height: u32,
    fb_stride: u32,
    x: u32,
    y: u32,
    value: u32,
    color: u32,
) {
    let v = value.min(999);
    let d0 = b'0' + ((v / 100) % 10) as u8;
    let d1 = b'0' + ((v / 10) % 10) as u8;
    let d2 = b'0' + (v % 10) as u8;
    draw_glyph_5x7(fb_vaddr, fb_width, fb_height, fb_stride, x, y, d0, color);
    draw_glyph_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        x + 6,
        y,
        d1,
        color,
    );
    draw_glyph_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        x + 12,
        y,
        d2,
        color,
    );
}

fn draw_u32_4(
    fb_vaddr: u64,
    fb_width: u32,
    fb_height: u32,
    fb_stride: u32,
    x: u32,
    y: u32,
    value: u32,
    color: u32,
) {
    let v = value.min(9999);
    let d0 = b'0' + ((v / 1000) % 10) as u8;
    let d1 = b'0' + ((v / 100) % 10) as u8;
    let d2 = b'0' + ((v / 10) % 10) as u8;
    let d3 = b'0' + (v % 10) as u8;
    draw_glyph_5x7(fb_vaddr, fb_width, fb_height, fb_stride, x, y, d0, color);
    draw_glyph_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        x + 6,
        y,
        d1,
        color,
    );
    draw_glyph_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        x + 12,
        y,
        d2,
        color,
    );
    draw_glyph_5x7(
        fb_vaddr,
        fb_width,
        fb_height,
        fb_stride,
        x + 18,
        y,
        d3,
        color,
    );
}

fn draw_glyph_5x7(
    fb_vaddr: u64,
    fb_width: u32,
    fb_height: u32,
    fb_stride: u32,
    x: u32,
    y: u32,
    ch: u8,
    color: u32,
) {
    let glyph = glyph_5x7(ch);
    let fb_ptr = fb_vaddr as *mut u32;

    for (row, bits) in glyph.iter().enumerate() {
        let py = y + row as u32;
        if py >= fb_height {
            continue;
        }
        let row_base = py as usize * fb_stride as usize;

        for col in 0..5u32 {
            let px = x + col;
            if px >= fb_width {
                continue;
            }
            if (bits & (1 << (4 - col))) != 0 {
                let idx = row_base + px as usize;
                unsafe {
                    *fb_ptr.add(idx) = color;
                }
            }
        }
    }
}

fn glyph_5x7(ch: u8) -> [u8; 7] {
    match ch {
        b'0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        b'1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        b'2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        b'3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        b'4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        b'5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        b'6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        b'7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        b'8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        b'9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        b'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        b'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        b'S' => [
            0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110,
        ],
        b'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        b'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        b'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        b'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        b'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        b'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        b'X' => [
            0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b01010, 0b10001,
        ],
        b':' => [0, 0b00100, 0, 0, 0, 0b00100, 0],
        b' ' => [0, 0, 0, 0, 0, 0, 0],
        _ => [0, 0, 0, 0, 0, 0, 0],
    }
}
