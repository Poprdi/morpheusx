use alloc::vec::Vec;

use crate::arena::Arena;
use crate::camera::Camera;
use crate::light::{FogMode, LightEnv};
use crate::math::fast;
use crate::math::fixed::Fx16;
use crate::math::mat4::Mat4;
use crate::math::trig::TrigTable;
use crate::math::vec::{Vec3, Vec4};
use crate::raster::clip::Clipper;
use crate::raster::edge::rasterize_triangle_to_spans;
use crate::raster::span::{Span, SpanGradients};
use crate::raster::triangle::Vertex;
use crate::scene::frustum::{CullResult, Frustum};
use crate::scene::mesh::Mesh;
use crate::target::{RenderTarget, TargetPixelFormat};
use crate::texture::mipmap::{MipChain, Texture};
use crate::texture::sample::{self, SampleMode};

/// Software renderer. Order: begin_frame, set_camera, draw_mesh×N, end_frame.
pub struct Pipeline {
    trig: TrigTable,
    clipper: Clipper,
    arena: Arena,
    spans: Vec<Span>,

    clip_verts: Vec<Vec4>,
    lit_colors: Vec<[f32; 3]>,

    view: Mat4,
    proj: Mat4,
    view_proj: Mat4,
    frustum: Frustum,
    camera_pos: Vec3,
    viewport_w: u32,
    viewport_h: u32,
    half_w: f32,
    half_h: f32,

    pub fog: FogMode,
    pub fog_color: [f32; 3],
    pub sample_mode: SampleMode,
    pub wireframe: bool,
    pub backface_cull: bool,
    pub depth_write: bool,

    pub stats: FrameStats,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStats {
    pub triangles_submitted: u32,
    pub triangles_culled: u32,
    pub triangles_clipped: u32,
    pub triangles_drawn: u32,
    pub pixels_written: u32,
    pub meshes_frustum_culled: u32,
}

pub struct Material<'a> {
    pub texture: Option<&'a MipChain>,
    pub specular_power: f32,
    /// Used if texture is None.
    pub base_color: [f32; 3],
}

impl<'a> Material<'a> {
    pub fn solid(r: f32, g: f32, b: f32) -> Self {
        Self {
            texture: None,
            specular_power: 0.0,
            base_color: [r, g, b],
        }
    }

    pub fn textured(mip: &'a MipChain) -> Self {
        Self {
            texture: Some(mip),
            specular_power: 16.0,
            base_color: [1.0, 1.0, 1.0],
        }
    }
}

impl Pipeline {
    pub fn new(viewport_w: u32, viewport_h: u32) -> Self {
        let max_spans = viewport_h as usize;
        Self {
            trig: TrigTable::new(),
            clipper: Clipper::new(),
            arena: Arena::new(4 * 1024 * 1024),
            spans: alloc::vec![Span::EMPTY; max_spans],
            clip_verts: Vec::with_capacity(256),
            lit_colors: Vec::with_capacity(256),
            view: Mat4::IDENTITY,
            proj: Mat4::IDENTITY,
            view_proj: Mat4::IDENTITY,
            frustum: Frustum {
                planes: [Vec4::ZERO; 6],
            },
            camera_pos: Vec3::ZERO,
            viewport_w,
            viewport_h,
            half_w: viewport_w as f32 * 0.5,
            half_h: viewport_h as f32 * 0.5,
            fog: FogMode::None,
            fog_color: [0.0, 0.0, 0.0],
            sample_mode: SampleMode::Bilinear,
            wireframe: false,
            backface_cull: true,
            depth_write: true,
            stats: FrameStats::default(),
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.viewport_w = w;
        self.viewport_h = h;
        self.half_w = w as f32 * 0.5;
        self.half_h = h as f32 * 0.5;
        if (h as usize) > self.spans.len() {
            self.spans.resize(h as usize, Span::EMPTY);
        }
    }

    pub fn trig(&self) -> &TrigTable {
        &self.trig
    }

    pub fn begin_frame(&mut self) {
        self.arena.reset();
        self.stats = FrameStats::default();
    }

    pub fn set_camera(&mut self, camera: &Camera) {
        self.view = camera.view_matrix(&self.trig);
        self.proj = camera.projection_matrix();
        self.view_proj = self.proj.mul(&self.view);
        self.frustum = Frustum::from_view_proj(&self.view_proj);
        self.camera_pos = camera.position;
    }

    /// Full vertex pipeline: transform, light, clip, project, rasterize, shade.
    pub fn draw_mesh(
        &mut self,
        mesh: &Mesh,
        model: &Mat4,
        material: &Material,
        lights: &LightEnv,
        target: &mut dyn RenderTarget,
    ) {
        // Bounding-sphere frustum cull (world space).
        let world_center = model.transform_point(mesh.bound_center).xyz();
        let scale_approx = {
            let sx2 = model.cols[0][0] * model.cols[0][0]
                + model.cols[0][1] * model.cols[0][1]
                + model.cols[0][2] * model.cols[0][2];
            let sy2 = model.cols[1][0] * model.cols[1][0]
                + model.cols[1][1] * model.cols[1][1]
                + model.cols[1][2] * model.cols[1][2];
            let sz2 = model.cols[2][0] * model.cols[2][0]
                + model.cols[2][1] * model.cols[2][1]
                + model.cols[2][2] * model.cols[2][2];

            let sx = if sx2 > 0.0 {
                sx2 * fast::inv_sqrt(sx2)
            } else {
                0.0
            };
            let sy = if sy2 > 0.0 {
                sy2 * fast::inv_sqrt(sy2)
            } else {
                0.0
            };
            let sz = if sz2 > 0.0 {
                sz2 * fast::inv_sqrt(sz2)
            } else {
                0.0
            };

            sx.max(sy).max(sz)
        };
        let world_radius = mesh.bound_radius * scale_approx;

        if self.frustum.test_sphere(world_center, world_radius) == CullResult::Outside {
            self.stats.meshes_frustum_culled += 1;
            return;
        }

        let mvp = self.view_proj.mul(model);
        let model_view = self.view.mul(model);
        let camera_pos = self.camera_pos;

        let vert_count = mesh.vertices.len();
        self.clip_verts.clear();
        self.lit_colors.clear();
        if self.clip_verts.capacity() < vert_count {
            self.clip_verts
                .reserve(vert_count - self.clip_verts.capacity());
        }
        if self.lit_colors.capacity() < vert_count {
            self.lit_colors
                .reserve(vert_count - self.lit_colors.capacity());
        }

        for mv in mesh.vertices.iter() {
            self.clip_verts.push(mvp.transform_point(mv.position));

            let world_pos = model.transform_point(mv.position).xyz();
            let world_normal = model.transform_dir(mv.normal).normalize();
            let view_dir = (camera_pos - world_pos).normalize();
            let vc = [
                mv.color[0] as f32 / 255.0,
                mv.color[1] as f32 / 255.0,
                mv.color[2] as f32 / 255.0,
            ];
            self.lit_colors.push(lights.evaluate(
                world_pos,
                world_normal,
                view_dir,
                material.specular_power,
                vc,
            ));
        }

        let idx = &mesh.indices;
        let tri_count = idx.len() / 3;

        for t in 0..tri_count {
            self.stats.triangles_submitted += 1;

            let i0 = idx[t * 3] as usize;
            let i1 = idx[t * 3 + 1] as usize;
            let i2 = idx[t * 3 + 2] as usize;

            if i0 >= vert_count || i1 >= vert_count || i2 >= vert_count {
                continue;
            }

            let clip_tri = [
                self.clip_verts[i0],
                self.clip_verts[i1],
                self.clip_verts[i2],
            ];

            if trivial_reject(&clip_tri) {
                self.stats.triangles_culled += 1;
                continue;
            }

            let build_vert = |vi: usize, clip: Vec4, cv: &[[f32; 3]]| -> Vertex {
                let mv = &mesh.vertices[vi];
                Vertex {
                    pos: clip,
                    color: cv[vi],
                    uv: mv.uv,
                    normal: mv.normal,
                    world_z: {
                        let eye = model_view.transform_point(mv.position);
                        -eye.z
                    },
                }
            };

            let tri_verts = [
                build_vert(i0, clip_tri[0], &self.lit_colors),
                build_vert(i1, clip_tri[1], &self.lit_colors),
                build_vert(i2, clip_tri[2], &self.lit_colors),
            ];

            let clipped = self.clipper.clip_triangle(&tri_verts);
            if clipped.len() < 3 {
                self.stats.triangles_culled += 1;
                continue;
            }
            if clipped.len() > 3 {
                self.stats.triangles_clipped += 1;
            }

            // Copy out to drop the clipper borrow before rasterizing.
            let clipped_count = clipped.len();
            let mut clipped_buf = [Vertex::ZEROED; 12];
            for (i, v) in clipped.iter().enumerate() {
                clipped_buf[i] = *v;
            }

            // Fan-triangulate.
            for fan_idx in 1..(clipped_count - 1) {
                let v0 = project_vertex(&clipped_buf[0], self.half_w, self.half_h);
                let v1 = project_vertex(&clipped_buf[fan_idx], self.half_w, self.half_h);
                let v2 = project_vertex(&clipped_buf[fan_idx + 1], self.half_w, self.half_h);

                if self.backface_cull {
                    let area = screen_area_2x(&v0, &v1, &v2);
                    if area <= 0.0 {
                        self.stats.triangles_culled += 1;
                        continue;
                    }
                }

                self.stats.triangles_drawn += 1;

                let span_count =
                    rasterize_triangle_to_spans(&[v0, v1, v2], &mut self.spans, self.viewport_h);

                let grads = SpanGradients::from_triangle(&v0, &v1, &v2);

                let format = target.pixel_format();
                let stride = target.stride();
                let vp_w = self.viewport_w;
                let dw = self.depth_write;
                let (color_buf, depth_buf) = target.buffers_mut();

                for s in 0..span_count {
                    self.stats.pixels_written += fill_span(
                        &self.spans[s],
                        &grads,
                        material,
                        &self.fog,
                        &self.fog_color,
                        self.sample_mode,
                        format,
                        stride,
                        vp_w,
                        dw,
                        color_buf,
                        depth_buf,
                    );
                }
            }
        }
    }

    pub fn end_frame(&mut self) {}
}

/// Perspective divide + viewport transform. Stores 1/w in pos.w for perspective-correct interp.
#[inline]
fn project_vertex(v: &Vertex, half_w: f32, half_h: f32) -> Vertex {
    let clip_w = v.pos.w;
    let safe_w = if clip_w.abs() < 1e-6 {
        if clip_w.is_sign_negative() {
            -1e-6
        } else {
            1e-6
        }
    } else {
        clip_w
    };

    let inv_w = 1.0 / safe_w;
    let ndc_x = v.pos.x * inv_w;
    let ndc_y = v.pos.y * inv_w;
    let ndc_z = v.pos.z * inv_w;

    let screen_x = (ndc_x + 1.0) * half_w;
    let screen_y = (1.0 - ndc_y) * half_h; // screen Y is down

    // Attributes pre-divided by w; rasterizer interpolates and recovers w.
    Vertex {
        pos: Vec4::new(screen_x, screen_y, ndc_z, inv_w),
        color: [v.color[0] * inv_w, v.color[1] * inv_w, v.color[2] * inv_w],
        uv: crate::math::vec::Vec2::new(v.uv.x * inv_w, v.uv.y * inv_w),
        normal: v.normal,
        world_z: v.world_z,
    }
}

/// 2× signed area of screen triangle (back-face cull sign).
#[inline]
fn screen_area_2x(v0: &Vertex, v1: &Vertex, v2: &Vertex) -> f32 {
    (v1.pos.x - v0.pos.x) * (v2.pos.y - v0.pos.y) - (v1.pos.y - v0.pos.y) * (v2.pos.x - v0.pos.x)
}

/// Cohen-Sutherland outcodes against the 6 clip planes; reject if all share a plane.
#[inline]
fn trivial_reject(tri: &[Vec4; 3]) -> bool {
    let outcode = |v: &Vec4| -> u8 {
        let mut c = 0u8;
        if v.x < -v.w {
            c |= 1;
        }
        if v.x > v.w {
            c |= 2;
        }
        if v.y < -v.w {
            c |= 4;
        }
        if v.y > v.w {
            c |= 8;
        }
        if v.z < 0.0 {
            c |= 16;
        } // near
        if v.z > v.w {
            c |= 32;
        } // far
        c
    };
    let c0 = outcode(&tri[0]);
    let c1 = outcode(&tri[1]);
    let c2 = outcode(&tri[2]);
    (c0 & c1 & c2) != 0
}

/// Dispatch to the cheapest valid span path.
fn fill_span(
    span: &Span,
    grads: &SpanGradients,
    material: &Material,
    fog: &FogMode,
    fog_color: &[f32; 3],
    sample_mode: SampleMode,
    format: TargetPixelFormat,
    stride: u32,
    vp_w: u32,
    depth_write: bool,
    color_buf: &mut [u32],
    depth_buf: &mut [u32],
) -> u32 {
    let is_solid = material.texture.is_none();
    let no_fog = matches!(fog, FogMode::None);

    if is_solid && no_fog {
        fill_span_solid(
            span,
            grads,
            material,
            format,
            stride,
            vp_w,
            depth_write,
            color_buf,
            depth_buf,
        )
    } else if !is_solid && no_fog {
        fill_span_textured(
            span,
            grads,
            material,
            sample_mode,
            format,
            stride,
            vp_w,
            depth_write,
            color_buf,
            depth_buf,
        )
    } else {
        fill_span_full(
            span,
            grads,
            material,
            fog,
            fog_color,
            sample_mode,
            format,
            stride,
            vp_w,
            depth_write,
            color_buf,
            depth_buf,
        )
    }
}

#[inline(always)]
fn pack_rgb_for_format(r: u8, g: u8, b: u8, format: TargetPixelFormat) -> u32 {
    match format {
        TargetPixelFormat::Bgrx => (b as u32) | ((g as u32) << 8) | ((r as u32) << 16),
        TargetPixelFormat::Rgbx => (r as u32) | ((g as u32) << 8) | ((b as u32) << 16),
        TargetPixelFormat::InternalRgba => {
            ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
        },
    }
}

/// Solid + no fog. Two divides per span, linear interp inside; pure 16.16 fixed inner loop.
fn fill_span_solid(
    span: &Span,
    grads: &SpanGradients,
    material: &Material,
    format: TargetPixelFormat,
    stride: u32,
    vp_w: u32,
    depth_write: bool,
    color_buf: &mut [u32],
    depth_buf: &mut [u32],
) -> u32 {
    let x_start = span.x_left.ceil().max(0) as u32;
    let x_end = span.x_right.ceil().min(vp_w as i32).max(0) as u32;
    if x_start >= x_end {
        return 0;
    }

    let x_start_fx = Fx16::from_i32(x_start as i32);
    let prestep_fx = x_start_fx - span.x_left;

    // 8.8 base color.
    let base_r = (material.base_color[0] * 256.0) as i32;
    let base_g = (material.base_color[1] * 256.0) as i32;
    let base_b = (material.base_color[2] * 256.0) as i32;

    // Divide at span endpoints; linear interp between.
    let inv_w_start = span.inv_w_left + grads.inv_w_step.mul(prestep_fx);
    let w_start = if inv_w_start.0 != 0 {
        Fx16::ONE.div(inv_w_start)
    } else {
        Fx16::ONE
    };

    let cr_start = (span.r_left + grads.r_step.mul(prestep_fx)).mul(w_start);
    let cg_start = (span.g_left + grads.g_step.mul(prestep_fx)).mul(w_start);
    let cb_start = (span.b_left + grads.b_step.mul(prestep_fx)).mul(w_start);

    let span_len_fx = Fx16::from_i32((x_end - x_start) as i32);
    let inv_w_end = inv_w_start + grads.inv_w_step.mul(span_len_fx);
    let w_end = if inv_w_end.0 != 0 {
        Fx16::ONE.div(inv_w_end)
    } else {
        Fx16::ONE
    };

    let cr_end = (span.r_left + grads.r_step.mul(prestep_fx + span_len_fx)).mul(w_end);
    let cg_end = (span.g_left + grads.g_step.mul(prestep_fx + span_len_fx)).mul(w_end);
    let cb_end = (span.b_left + grads.b_step.mul(prestep_fx + span_len_fx)).mul(w_end);

    let len = (x_end - x_start) as i32;
    let len_fx = Fx16::from_i32(len.max(1));
    let r_step = (cr_end - cr_start).div(len_fx);
    let g_step = (cg_end - cg_start).div(len_fx);
    let b_step = (cb_end - cb_start).div(len_fx);

    let mut cr = cr_start;
    let mut cg = cg_start;
    let mut cb = cb_start;
    let mut z = span.z_left + grads.z_step.mul(prestep_fx);

    let row_offset = (span.y * stride) as usize;
    let buf_end = color_buf.len().min(depth_buf.len());
    let mut pixels_written = 0u32;

    for x in x_start..x_end {
        let buf_idx = row_offset + x as usize;
        if buf_idx >= buf_end {
            break;
        }

        let depth = if z.0 < 0 { 0 } else { z.0 as u32 };
        if depth >= depth_buf[buf_idx] {
            cr += r_step;
            cg += g_step;
            cb += b_step;
            z += grads.z_step;
            continue;
        }
        if depth_write {
            depth_buf[buf_idx] = depth;
        }

        let pr = fast::clamp_u8((cr.0 * base_r) >> 16);
        let pg = fast::clamp_u8((cg.0 * base_g) >> 16);
        let pb = fast::clamp_u8((cb.0 * base_b) >> 16);
        color_buf[buf_idx] = pack_rgb_for_format(pr, pg, pb, format);
        pixels_written += 1;

        cr += r_step;
        cg += g_step;
        cb += b_step;
        z += grads.z_step;
    }

    pixels_written
}

/// Pixels per perspective divide in textured path (classic Quake affine step).
const AFFINE_STEP: u32 = 8;

/// Textured, no fog. Divide every AFFINE_STEP px; linear UV inside.
fn fill_span_textured(
    span: &Span,
    grads: &SpanGradients,
    material: &Material,
    sample_mode: SampleMode,
    format: TargetPixelFormat,
    stride: u32,
    vp_w: u32,
    depth_write: bool,
    color_buf: &mut [u32],
    depth_buf: &mut [u32],
) -> u32 {
    let x_start = span.x_left.ceil().max(0) as u32;
    let x_end = span.x_right.ceil().min(vp_w as i32).max(0) as u32;
    if x_start >= x_end {
        return 0;
    }

    let mip = match material.texture {
        Some(m) => m,
        None => return 0,
    };
    let tex = mip.level(0);

    let x_start_fx = Fx16::from_i32(x_start as i32);
    let prestep_fx = x_start_fx - span.x_left;

    let mut inv_w = span.inv_w_left + grads.inv_w_step.mul(prestep_fx);
    let mut cr_iw = span.r_left + grads.r_step.mul(prestep_fx);
    let mut cg_iw = span.g_left + grads.g_step.mul(prestep_fx);
    let mut cb_iw = span.b_left + grads.b_step.mul(prestep_fx);
    let mut tu_iw = span.u_left + grads.u_step.mul(prestep_fx);
    let mut tv_iw = span.v_left + grads.v_step.mul(prestep_fx);
    let mut z = span.z_left + grads.z_step.mul(prestep_fx);

    let row_offset = (span.y * stride) as usize;
    let buf_end = color_buf.len().min(depth_buf.len());
    let mut pixels_written = 0u32;

    let mut x = x_start;
    while x < x_end {
        let chunk_end = (x + AFFINE_STEP).min(x_end);
        let chunk_len = chunk_end - x;

        let w0 = if inv_w.0 != 0 {
            Fx16::ONE.div(inv_w)
        } else {
            Fx16::ONE
        };
        let u0 = tu_iw.mul(w0);
        let v0 = tv_iw.mul(w0);
        let r0 = cr_iw.mul(w0);
        let g0 = cg_iw.mul(w0);
        let b0 = cb_iw.mul(w0);

        let inv_w_next = inv_w + Fx16(grads.inv_w_step.0 * chunk_len as i32);
        let w1 = if inv_w_next.0 != 0 {
            Fx16::ONE.div(inv_w_next)
        } else {
            Fx16::ONE
        };
        let tu_next = tu_iw + Fx16(grads.u_step.0 * chunk_len as i32);
        let tv_next = tv_iw + Fx16(grads.v_step.0 * chunk_len as i32);
        let u1 = tu_next.mul(w1);
        let v1 = tv_next.mul(w1);
        let cr_next = cr_iw + Fx16(grads.r_step.0 * chunk_len as i32);
        let cg_next = cg_iw + Fx16(grads.g_step.0 * chunk_len as i32);
        let cb_next = cb_iw + Fx16(grads.b_step.0 * chunk_len as i32);
        let r1 = cr_next.mul(w1);
        let g1 = cg_next.mul(w1);
        let b1 = cb_next.mul(w1);

        let inv_len = Fx16::from_i32((chunk_len as i32).max(1));
        let du = (u1 - u0).div(inv_len);
        let dv = (v1 - v0).div(inv_len);
        let dr = (r1 - r0).div(inv_len);
        let dg = (g1 - g0).div(inv_len);
        let db = (b1 - b0).div(inv_len);

        let mut u_cur = u0;
        let mut v_cur = v0;
        let mut r_cur = r0;
        let mut g_cur = g0;
        let mut b_cur = b0;

        for px in x..chunk_end {
            let buf_idx = row_offset + px as usize;
            if buf_idx >= buf_end {
                break;
            }

            let depth = if z.0 < 0 { 0 } else { z.0 as u32 };
            if depth >= depth_buf[buf_idx] {
                u_cur += du;
                v_cur += dv;
                r_cur += dr;
                g_cur += dg;
                b_cur += db;
                z += grads.z_step;
                continue;
            }
            if depth_write {
                depth_buf[buf_idx] = depth;
            }

            let texel = match sample_mode {
                SampleMode::Nearest => sample::sample_nearest(tex, u_cur.0, v_cur.0),
                SampleMode::Bilinear => sample::sample_bilinear(tex, u_cur.0, v_cur.0),
            };
            let (tr, tg, tb, _ta) = Texture::unpack(texel);

            // Fx16 × u8 → 16.16 i32, >> 16 → u8.
            let pr = fast::clamp_u8((r_cur.0 * tr as i32) >> 16);
            let pg = fast::clamp_u8((g_cur.0 * tg as i32) >> 16);
            let pb = fast::clamp_u8((b_cur.0 * tb as i32) >> 16);
            color_buf[buf_idx] = pack_rgb_for_format(pr, pg, pb, format);
            pixels_written += 1;

            u_cur += du;
            v_cur += dv;
            r_cur += dr;
            g_cur += dg;
            b_cur += db;
            z += grads.z_step;
        }

        inv_w += Fx16(grads.inv_w_step.0 * chunk_len as i32);
        cr_iw += Fx16(grads.r_step.0 * chunk_len as i32);
        cg_iw += Fx16(grads.g_step.0 * chunk_len as i32);
        cb_iw += Fx16(grads.b_step.0 * chunk_len as i32);
        tu_iw += Fx16(grads.u_step.0 * chunk_len as i32);
        tv_iw += Fx16(grads.v_step.0 * chunk_len as i32);
        x = chunk_end;
    }

    pixels_written
}

/// Fallback: per-pixel divide, supports fog.
fn fill_span_full(
    span: &Span,
    grads: &SpanGradients,
    material: &Material,
    fog: &FogMode,
    fog_color: &[f32; 3],
    sample_mode: SampleMode,
    format: TargetPixelFormat,
    stride: u32,
    vp_w: u32,
    depth_write: bool,
    color_buf: &mut [u32],
    depth_buf: &mut [u32],
) -> u32 {
    let x_start = span.x_left.ceil().max(0) as u32;
    let x_end = span.x_right.ceil().min(vp_w as i32).max(0) as u32;
    if x_start >= x_end {
        return 0;
    }

    let x_start_fx = Fx16::from_i32(x_start as i32);
    let prestep_fx = x_start_fx - span.x_left;

    let mut inv_w = span.inv_w_left + grads.inv_w_step.mul(prestep_fx);
    let mut cr = span.r_left + grads.r_step.mul(prestep_fx);
    let mut cg = span.g_left + grads.g_step.mul(prestep_fx);
    let mut cb = span.b_left + grads.b_step.mul(prestep_fx);
    let mut tu = span.u_left + grads.u_step.mul(prestep_fx);
    let mut tv = span.v_left + grads.v_step.mul(prestep_fx);
    let mut z = span.z_left + grads.z_step.mul(prestep_fx);
    let mut fog_val = span.fog_left + grads.fog_step.mul(prestep_fx);

    let row_offset = (span.y * stride) as usize;
    let buf_end = color_buf.len().min(depth_buf.len());
    let mut pixels_written = 0u32;

    for x in x_start..x_end {
        let buf_idx = row_offset + x as usize;
        if buf_idx >= buf_end {
            break;
        }

        let depth = if z.0 < 0 { 0 } else { z.0 as u32 };
        if depth >= depth_buf[buf_idx] {
            inv_w += grads.inv_w_step;
            cr += grads.r_step;
            cg += grads.g_step;
            cb += grads.b_step;
            tu += grads.u_step;
            tv += grads.v_step;
            z += grads.z_step;
            fog_val += grads.fog_step;
            continue;
        }
        if depth_write {
            depth_buf[buf_idx] = depth;
        }

        let w = if inv_w.0 != 0 {
            Fx16::ONE.div(inv_w)
        } else {
            Fx16::ONE
        };

        let r_f = cr.mul(w);
        let g_f = cg.mul(w);
        let b_f = cb.mul(w);

        let (tex_r, tex_g, tex_b) = if let Some(mip) = material.texture {
            let u_px = tu.mul(w);
            let v_px = tv.mul(w);
            let tex = mip.level(0);
            let texel = match sample_mode {
                SampleMode::Nearest => sample::sample_nearest(tex, u_px.0, v_px.0),
                SampleMode::Bilinear => sample::sample_bilinear(tex, u_px.0, v_px.0),
            };
            let (tr, tg, tb, _ta) = Texture::unpack(texel);
            (tr as f32 / 255.0, tg as f32 / 255.0, tb as f32 / 255.0)
        } else {
            (
                material.base_color[0],
                material.base_color[1],
                material.base_color[2],
            )
        };

        let mut out_r = r_f.to_f32() * tex_r;
        let mut out_g = g_f.to_f32() * tex_g;
        let mut out_b = b_f.to_f32() * tex_b;

        match *fog {
            FogMode::None => {},
            _ => {
                let fog_factor = fog.compute(fog_val.to_f32());
                out_r = out_r * fog_factor + fog_color[0] * (1.0 - fog_factor);
                out_g = out_g * fog_factor + fog_color[1] * (1.0 - fog_factor);
                out_b = out_b * fog_factor + fog_color[2] * (1.0 - fog_factor);
            },
        }

        let pr = fast::clamp_u8((out_r * 255.0) as i32);
        let pg = fast::clamp_u8((out_g * 255.0) as i32);
        let pb = fast::clamp_u8((out_b * 255.0) as i32);
        color_buf[buf_idx] = pack_rgb_for_format(pr, pg, pb, format);
        pixels_written += 1;

        inv_w += grads.inv_w_step;
        cr += grads.r_step;
        cg += grads.g_step;
        cb += grads.b_step;
        tu += grads.u_step;
        tv += grads.v_step;
        z += grads.z_step;
        fog_val += grads.fog_step;
    }

    pixels_written
}
