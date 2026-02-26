#![no_std]
#![no_main]
extern crate alloc;

use alloc::vec::Vec;
use libmorpheus::entry;
use libmorpheus::hw::{fb_info, fb_lock, fb_map};
use libmorpheus::time;
use morpheus_gfx3d::pipeline::Pipeline;
use morpheus_gfx3d::scene::mesh::Mesh;
use morpheus_gfx3d::math::mat4::Mat4;
use morpheus_gfx3d::math::vec::Vec3;
use morpheus_gfx3d::math::trig::TrigTable;
use morpheus_gfx3d::light::{LightEnv, DirLight, PointLight};
use morpheus_gfx3d::camera::Camera;
use morpheus_gfx3d::target::{TargetPixelFormat, SoftwareTarget};
use morpheus_gfx3d::pipeline::Material;

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

    // ── Save current framebuffer contents before we take over ──
    let _saved_framebuffer = save_framebuffer(fb_vaddr, fb_width, fb_height, fb_stride);

    // ── Clear framebuffer to black ──
    clear_framebuffer(fb_vaddr, fb_width, fb_height, fb_stride, 0x00000000);

    // ── Create software render target (intermediate buffer) ──
    let mut target = SoftwareTarget::new(fb_width, fb_height, fb_format);

    // ── Initialize 3D pipeline ──
    let mut pipeline = Pipeline::new(fb_width, fb_height);
    pipeline.fog = morpheus_gfx3d::light::FogMode::None;
    pipeline.wireframe = false;
    pipeline.backface_cull = true;
    pipeline.sample_mode = morpheus_gfx3d::texture::sample::SampleMode::Nearest;

    // ── Set up lighting ──
    let mut lights = LightEnv::new();
    lights.ambient = [0.4, 0.4, 0.4];
    lights.dir_lights.push(DirLight {
        direction: Vec3::new(0.577, 0.577, -0.577).normalize(),
        color: [1.0, 1.0, 1.0],
    });
    lights.point_lights.push(PointLight::new(
        Vec3::new(5.0, 5.0, 5.0),
        [1.0, 0.8, 0.6],
        20.0,
    ));

    // ── Create cube mesh ──
    let cube = Mesh::cube();

    // ── Set up camera (looking at cube) ──
    let aspect = fb_width as f32 / fb_height.max(1) as f32;
    let mut camera = Camera::new(aspect);
    camera.position = Vec3::new(0.0, 0.0, 5.0);
    camera.yaw = 0.0;
    camera.pitch = 0.0;

    // ── Trig table for rotation ──
    let trig = TrigTable::new();

    // ── Render loop (spin cube forever, until Ctrl+C) ──
    loop {
        // Update target state (reset for new frame)
        target.clear(0x00000000); // Black background
        pipeline.begin_frame();
        pipeline.set_camera(&camera);

        // Cube model matrix: rotate around Y axis based on frame
        let nanos = time::clock_gettime();
        let phase = (nanos % 1_000_000_000) as f32 / 1_000_000_000.0;
        let angle_rad = phase * 6.2831855; // 2π rad -> exactly 1 rotation per second
        let (sin_a, cos_a) = trig.sin_cos(angle_rad);

        let model = Mat4::rotation_y(sin_a, cos_a)
            .mul(&Mat4::translation(0.0, 0.0, 0.0));

        // Material: solid green
        let material = Material::solid(0.3, 0.8, 0.3);

        // Draw the cube
        pipeline.draw_mesh(&cube, &model, &material, &lights, &mut target);

        // Copy render target to hardware framebuffer
        blit_to_hardware(&target, fb_vaddr, fb_width, fb_height, fb_stride, fb_format);

    }

    // Unreachable, but restore framebuffer on exit for safety
    #[allow(unreachable_code)]
    {
        restore_framebuffer(fb_vaddr, fb_width, fb_height, fb_stride, &_saved_framebuffer);
        0
    }
}

/// Blit the intermediate software target to hardware framebuffer.
fn blit_to_hardware(
    target: &SoftwareTarget,
    fb_vaddr: u64,
    fb_width: u32,
    fb_height: u32,
    fb_stride: u32,
    _fb_format: TargetPixelFormat,
) {
    // Safety: fb_vaddr was returned by fb_map() syscall, so it's a valid mapped region
    // that we own. We treat it as a mutable u32 array and copy our rendered frame to it.
    let fb_ptr = fb_vaddr as *mut u32;

    // Copy each row from render target to hardware framebuffer
    for y in 0..fb_height {
        let src_row = y as usize * fb_width as usize;
        let dst_row = y as usize * fb_stride as usize;

        unsafe {
            for x in 0..fb_width {
                let src_idx = src_row + x as usize;
                let dst_idx = dst_row + x as usize;

                if src_idx < target.color.len() {
                    *fb_ptr.add(dst_idx) = target.color[src_idx];
                }
            }
        }
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

/// Save the current framebuffer contents to a heap-allocated Vec.
fn save_framebuffer(fb_vaddr: u64, fb_width: u32, fb_height: u32, fb_stride: u32) -> Vec<u32> {
    let fb_ptr = fb_vaddr as *const u32;
    let mut saved = Vec::with_capacity((fb_width * fb_height) as usize);
    
    for y in 0..fb_height {
        let src_row = y as usize * fb_stride as usize;
        for x in 0..fb_width {
            let idx = src_row + x as usize;
            unsafe {
                saved.push(*fb_ptr.add(idx));
            }
        }
    }
    
    saved
}

/// Restore a previously saved framebuffer.
fn restore_framebuffer(fb_vaddr: u64, fb_width: u32, fb_height: u32, fb_stride: u32, saved: &[u32]) {
    let fb_ptr = fb_vaddr as *mut u32;
    let mut saved_idx = 0usize;
    
    for y in 0..fb_height {
        let dst_row = y as usize * fb_stride as usize;
        for x in 0..fb_width {
            let idx = dst_row + x as usize;
            if saved_idx < saved.len() {
                unsafe {
                    *fb_ptr.add(idx) = saved[saved_idx];
                }
                saved_idx += 1;
            }
        }
    }
}
