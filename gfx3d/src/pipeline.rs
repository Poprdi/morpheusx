use alloc::vec::Vec;

use crate::math::vec::{Vec3, Vec4};
use crate::math::mat4::Mat4;
use crate::math::fixed::Fx16;
use crate::math::fast;
use crate::raster::triangle::Vertex;
use crate::raster::clip::Clipper;
use crate::raster::span::{Span, SpanGradients};
use crate::raster::edge::rasterize_triangle_to_spans;
use crate::texture::mipmap::{Texture, MipChain};
use crate::texture::sample::{self, SampleMode};
use crate::light::{LightEnv, FogMode};
use crate::scene::mesh::Mesh;
use crate::scene::frustum::{Frustum, CullResult};
use crate::target::{RenderTarget, TargetPixelFormat, convert_pixel};
use crate::arena::Arena;
use crate::camera::Camera;
use crate::math::trig::TrigTable;

/// The rendering pipeline — Quake-class software renderer.
///
/// Call order per frame:
/// 1. `begin_frame()` — clear buffers, reset arena
/// 2. `set_camera()` — update view/projection
/// 3. `draw_mesh()` × N — submit geometry with transforms and materials
/// 4. `end_frame()` — finalize (optional post-processing)
///
/// Internally, each `draw_mesh` call:
/// - Frustum-culls the mesh's bounding sphere
/// - Transforms vertices to clip space (model → world → clip)
/// - Clips triangles against the 6 frustum planes
/// - Performs perspective divide + viewport transform
/// - Rasterizes to spans with fixed-point edge walking
/// - Fills spans with perspective-correct texturing + lighting
pub struct Pipeline {
    trig: TrigTable,
    clipper: Clipper,
    arena: Arena,
    spans: Vec<Span>,

    // Cached per-frame state
    view: Mat4,
    proj: Mat4,
    view_proj: Mat4,
    frustum: Frustum,
    camera_pos: Vec3,
    viewport_w: u32,
    viewport_h: u32,
    half_w: f32,
    half_h: f32,

    // Rendering modes
    pub fog: FogMode,
    pub fog_color: [f32; 3],
    pub sample_mode: SampleMode,
    pub wireframe: bool,
    pub backface_cull: bool,

    // Stats
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

/// Material description for a draw call.
pub struct Material<'a> {
    pub texture: Option<&'a MipChain>,
    pub specular_power: f32,
    pub base_color: [f32; 3], // fallback color if no texture
}

impl<'a> Material<'a> {
    pub fn solid(r: f32, g: f32, b: f32) -> Self {
        Self { texture: None, specular_power: 0.0, base_color: [r, g, b] }
    }

    pub fn textured(mip: &'a MipChain) -> Self {
        Self { texture: Some(mip), specular_power: 16.0, base_color: [1.0, 1.0, 1.0] }
    }
}

impl Pipeline {
    pub fn new(viewport_w: u32, viewport_h: u32) -> Self {
        let max_spans = viewport_h as usize; // max spans per triangle = viewport height
        Self {
            trig: TrigTable::new(),
            clipper: Clipper::new(),
            arena: Arena::new(4 * 1024 * 1024), // 4MB per-frame scratch
            spans: alloc::vec![Span::EMPTY; max_spans],
            view: Mat4::IDENTITY,
            proj: Mat4::IDENTITY,
            view_proj: Mat4::IDENTITY,
            frustum: Frustum { planes: [Vec4::ZERO; 6] },
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
            stats: FrameStats::default(),
        }
    }

    /// Resize viewport (call when framebuffer resolution changes).
    pub fn resize(&mut self, w: u32, h: u32) {
        self.viewport_w = w;
        self.viewport_h = h;
        self.half_w = w as f32 * 0.5;
        self.half_h = h as f32 * 0.5;
        if (h as usize) > self.spans.len() {
            self.spans.resize(h as usize, Span::EMPTY);
        }
    }

    pub fn trig(&self) -> &TrigTable { &self.trig }

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

    /// Submit a mesh with a model-to-world transform.
    ///
    /// This is the main draw call. Handles the full vertex pipeline:
    /// transform → light → clip → project → rasterize → shade.
    pub fn draw_mesh(
        &mut self,
        mesh: &Mesh,
        model: &Mat4,
        material: &Material,
        lights: &LightEnv,
        target: &mut dyn RenderTarget,
    ) {
        // ── 1. Frustum cull (bounding sphere in world space) ──
        let world_center = model.transform_point(mesh.bound_center).xyz();
        let scale_approx = {
            let sx = model.cols[0][0] * model.cols[0][0] + model.cols[0][1] * model.cols[0][1] + model.cols[0][2] * model.cols[0][2];
            sx * fast::inv_sqrt(sx)
        };
        let world_radius = mesh.bound_radius * scale_approx;

        if self.frustum.test_sphere(world_center, world_radius) == CullResult::Outside {
            self.stats.meshes_frustum_culled += 1;
            return;
        }

        // ── 2. Compute combined model-view-projection ──
        let mvp = self.view_proj.mul(model);
        let model_view = self.view.mul(model);
        let camera_pos = self.camera_pos;

        // ── 3. Transform + light all vertices ──
        let vert_count = mesh.vertices.len();
        let mut clip_verts = Vec::with_capacity(vert_count);
        let mut lit_colors = Vec::with_capacity(vert_count);

        for mv in mesh.vertices.iter() {
            clip_verts.push(mvp.transform_point(mv.position));

            let world_pos = model.transform_point(mv.position).xyz();
            let world_normal = model.transform_dir(mv.normal).normalize();
            let view_dir = (camera_pos - world_pos).normalize();
            let vc = [
                mv.color[0] as f32 / 255.0,
                mv.color[1] as f32 / 255.0,
                mv.color[2] as f32 / 255.0,
            ];
            lit_colors.push(lights.evaluate(
                world_pos, world_normal, view_dir,
                material.specular_power, vc,
            ));
        }

        // ── 4. Process each triangle ──
        let idx = &mesh.indices;
        let tri_count = idx.len() / 3;

        for t in 0..tri_count {
            self.stats.triangles_submitted += 1;

            let i0 = idx[t * 3] as usize;
            let i1 = idx[t * 3 + 1] as usize;
            let i2 = idx[t * 3 + 2] as usize;

            if i0 >= vert_count || i1 >= vert_count || i2 >= vert_count { continue; }

            let clip_tri = [clip_verts[i0], clip_verts[i1], clip_verts[i2]];

            if trivial_reject(&clip_tri) {
                self.stats.triangles_culled += 1;
                continue;
            }

            let build_vert = |vi: usize, clip: Vec4| -> Vertex {
                let mv = &mesh.vertices[vi];
                Vertex {
                    pos: clip,
                    color: lit_colors[vi],
                    uv: mv.uv,
                    normal: mv.normal,
                    world_z: {
                        let eye = model_view.transform_point(mv.position);
                        -eye.z
                    },
                }
            };

            let tri_verts = [
                build_vert(i0, clip_tri[0]),
                build_vert(i1, clip_tri[1]),
                build_vert(i2, clip_tri[2]),
            ];

            // ── 5. Clip against frustum ──
            let clipped = self.clipper.clip_triangle(&tri_verts);
            if clipped.len() < 3 {
                self.stats.triangles_culled += 1;
                continue;
            }
            if clipped.len() > 3 { self.stats.triangles_clipped += 1; }

            // Copy clipped verts out so we can release the clipper borrow
            let clipped_count = clipped.len();
            let mut clipped_buf = [Vertex::ZEROED; 12];
            for (i, v) in clipped.iter().enumerate() {
                clipped_buf[i] = *v;
            }

            // ── 6. Fan-triangulate clipped polygon and rasterize ──
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

                let span_count = rasterize_triangle_to_spans(
                    &[v0, v1, v2],
                    &mut self.spans,
                    self.viewport_h,
                );

                let format = target.pixel_format();
                let stride = target.stride();
                let vp_w = self.viewport_w;
                let (color_buf, depth_buf) = target.buffers_mut();

                for s in 0..span_count {
                    self.stats.pixels_written += fill_span(
                        &self.spans[s], material, &self.fog, &self.fog_color,
                        self.sample_mode, format, stride, vp_w, color_buf, depth_buf,
                    );
                }
            }
        }
    }

    /// Fill a single scanline span with shaded pixels.
    ///
    /// This is THE hot inner loop — every optimization matters here.
    pub fn end_frame(&mut self) {
        // Reserved for post-processing passes (gamma correction, etc.)
    }
}

/// Perspective divide + viewport transform a clip-space vertex to screen space.
///
/// After this, vertex.pos = (screen_x, screen_y, depth_01, 1/clip_w).
/// The 1/clip_w is stored in pos.w for perspective-correct interpolation.
#[inline]
fn project_vertex(v: &Vertex, half_w: f32, half_h: f32) -> Vertex {
    let clip_w = v.pos.w;
    if clip_w.abs() < 1e-6 { return *v; }

    let inv_w = 1.0 / clip_w;
    let ndc_x = v.pos.x * inv_w;
    let ndc_y = v.pos.y * inv_w;
    let ndc_z = v.pos.z * inv_w;

    // Viewport transform: NDC [-1,1] → screen pixels
    let screen_x = (ndc_x + 1.0) * half_w;
    let screen_y = (1.0 - ndc_y) * half_h; // Y flipped (screen Y goes down)

    // Pre-divide attributes by clip_w for perspective-correct interpolation
    Vertex {
        pos: Vec4::new(screen_x, screen_y, ndc_z, inv_w),
        color: [v.color[0] * inv_w, v.color[1] * inv_w, v.color[2] * inv_w],
        uv: crate::math::vec::Vec2::new(v.uv.x * inv_w, v.uv.y * inv_w),
        normal: v.normal,
        world_z: v.world_z,
    }
}

/// Signed area × 2 of screen-space triangle (for back-face culling).
#[inline]
fn screen_area_2x(v0: &Vertex, v1: &Vertex, v2: &Vertex) -> f32 {
    (v1.pos.x - v0.pos.x) * (v2.pos.y - v0.pos.y) -
    (v1.pos.y - v0.pos.y) * (v2.pos.x - v0.pos.x)
}

/// Trivial reject: true if all 3 vertices are outside the same clip plane.
///
/// Cohen-Sutherland-style outcodes for 3D clip space.
#[inline]
fn trivial_reject(tri: &[Vec4; 3]) -> bool {
    let outcode = |v: &Vec4| -> u8 {
        let mut c = 0u8;
        if v.x < -v.w { c |= 1; }
        if v.x >  v.w { c |= 2; }
        if v.y < -v.w { c |= 4; }
        if v.y >  v.w { c |= 8; }
        if v.z <  0.0 { c |= 16; } // near (reversed-Z)
        if v.z >  v.w { c |= 32; } // far
        c
    };
    let c0 = outcode(&tri[0]);
    let c1 = outcode(&tri[1]);
    let c2 = outcode(&tri[2]);
    (c0 & c1 & c2) != 0
}

/// Inner pixel loop — process one span (horizontal scanline segment).
///
/// Extracted as a free function to avoid borrow conflicts with Pipeline fields.
fn fill_span(
    span: &Span,
    material: &Material,
    fog: &FogMode,
    fog_color: &[f32; 3],
    sample_mode: SampleMode,
    format: TargetPixelFormat,
    stride: u32,
    vp_w: u32,
    color_buf: &mut [u32],
    depth_buf: &mut [u32],
) -> u32 {
    let grads = SpanGradients::from_span(span);
    let x_start = span.x_left.ceil().max(0) as u32;
    let x_end = span.x_right.ceil().min(vp_w as i32).max(0) as u32;
    if x_start >= x_end { return 0; }

    let prestep = x_start as i32 - span.x_left.ceil();
    let prestep_fx = Fx16::from_i32(prestep);

    let mut inv_w = span.inv_w_left + grads.inv_w_step.mul(prestep_fx);
    let mut cr = span.r_left + grads.r_step.mul(prestep_fx);
    let mut cg = span.g_left + grads.g_step.mul(prestep_fx);
    let mut cb = span.b_left + grads.b_step.mul(prestep_fx);
    let mut tu = span.u_left + grads.u_step.mul(prestep_fx);
    let mut tv = span.v_left + grads.v_step.mul(prestep_fx);
    let mut z = span.z_left + grads.z_step.mul(prestep_fx);
    let mut fog_val = span.fog_left + grads.fog_step.mul(prestep_fx);

    let row_offset = (span.y * stride) as usize;
    let mut pixels_written = 0u32;

    for x in x_start..x_end {
        let buf_idx = row_offset + x as usize;
        if buf_idx >= color_buf.len() || buf_idx >= depth_buf.len() { break; }

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
        depth_buf[buf_idx] = depth;

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
            let level = 0usize;
            let tex = mip.level(level);
            let u_fx = u_px.0;
            let v_fx = v_px.0;
            let texel = match sample_mode {
                SampleMode::Nearest => sample::sample_nearest(tex, u_fx, v_fx),
                SampleMode::Bilinear => sample::sample_bilinear(tex, u_fx, v_fx),
            };
            let (tr, tg, tb, _ta) = Texture::unpack(texel);
            (tr as f32 / 255.0, tg as f32 / 255.0, tb as f32 / 255.0)
        } else {
            (material.base_color[0], material.base_color[1], material.base_color[2])
        };

        let mut out_r = r_f.to_f32() * tex_r;
        let mut out_g = g_f.to_f32() * tex_g;
        let mut out_b = b_f.to_f32() * tex_b;

        match *fog {
            FogMode::None => {}
            _ => {
                let fog_factor = fog.compute(fog_val.to_f32());
                out_r = out_r * fog_factor + fog_color[0] * (1.0 - fog_factor);
                out_g = out_g * fog_factor + fog_color[1] * (1.0 - fog_factor);
                out_b = out_b * fog_factor + fog_color[2] * (1.0 - fog_factor);
            }
        }

        let pr = fast::clamp_u8((out_r * 255.0) as i32);
        let pg = fast::clamp_u8((out_g * 255.0) as i32);
        let pb = fast::clamp_u8((out_b * 255.0) as i32);
        let internal = Texture::pack(pr, pg, pb, 255);
        color_buf[buf_idx] = convert_pixel(internal, format);
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
