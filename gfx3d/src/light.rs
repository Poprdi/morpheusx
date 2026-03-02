use crate::math::fast::saturate;
use crate::math::vec::Vec3;
use alloc::vec::Vec;

/// Quake-style lighting: per-vertex Gouraud with Phong-like specular approximation.
///
/// The key optimization: all lighting is computed PER VERTEX in world/view space,
/// then interpolated across triangles during rasterization. This means the lighting
/// cost scales with vertex count (50K), not pixel count (786K at 1024×768).
///
/// For static geometry (walls, floors), lighting is baked into lightmaps at map
/// compile time and combined via the surface cache.

/// A directional light (sun, moon).
#[derive(Debug, Clone, Copy)]
pub struct DirLight {
    pub direction: Vec3, // normalized, pointing TOWARD the light source
    pub color: [f32; 3], // RGB intensity [0, 1+] (can be >1 for HDR-ish overbrightening)
}

/// A point light (torch, explosion flash, muzzle flash).
#[derive(Debug, Clone, Copy)]
pub struct PointLight {
    pub position: Vec3,
    pub color: [f32; 3],
    pub radius: f32,     // attenuation distance (light = 0 beyond radius)
    pub inv_radius: f32, // pre-computed 1/radius for fast attenuation
}

impl PointLight {
    pub fn new(position: Vec3, color: [f32; 3], radius: f32) -> Self {
        Self {
            position,
            color,
            radius,
            inv_radius: if radius > 0.0 { 1.0 / radius } else { 0.0 },
        }
    }
}

/// Ambient + directional + point light accumulation.
///
/// All-at-once per-vertex: avoids per-light function call overhead.
/// This processes up to MAX_LIGHTS in a single pass per vertex.
pub struct LightEnv {
    pub ambient: [f32; 3],
    pub dir_lights: Vec<DirLight>,
    pub point_lights: Vec<PointLight>,
}

/// Maximum lights evaluated per vertex (hard limit for performance).
///
/// Beyond 4 point lights: sort by distance, only evaluate closest 4.
/// Quake 2 used exactly 4; Quake 3 used 1 directional + 4 point.
pub const MAX_POINT_LIGHTS: usize = 4;

impl LightEnv {
    pub fn new() -> Self {
        Self {
            ambient: [0.05, 0.05, 0.05],
            dir_lights: Vec::new(),
            point_lights: Vec::new(),
        }
    }

    /// Compute per-vertex lighting color.
    ///
    /// Uses N·L diffuse + half-angle specular (Blinn-Phong approximation).
    /// The Blinn-Phong half-vector trick: instead of computing reflection
    /// vector R = 2(N·L)N - L (expensive), compute H = normalize(L + V),
    /// then specular = (N·H)^power. Same visual result, one fewer normalize.
    ///
    /// Returns RGB [0, 1+] (unclamped — gets clamped when converting to u8).
    #[inline]
    pub fn evaluate(
        &self,
        world_pos: Vec3,
        normal: Vec3,
        view_dir: Vec3,      // normalized direction from vertex to camera
        specular_power: f32, // 0 = no specular, 16-64 for glossy surfaces
        vertex_color: [f32; 3],
    ) -> [f32; 3] {
        let mut r = self.ambient[0];
        let mut g = self.ambient[1];
        let mut b = self.ambient[2];

        // Directional lights
        for dl in &self.dir_lights {
            let n_dot_l = normal.dot(dl.direction);
            if n_dot_l <= 0.0 {
                continue;
            }
            let diff = saturate(n_dot_l);

            r += dl.color[0] * diff;
            g += dl.color[1] * diff;
            b += dl.color[2] * diff;

            if specular_power > 0.0 {
                let half = (dl.direction + view_dir).normalize();
                let n_dot_h = saturate(normal.dot(half));
                let spec = pow_approx(n_dot_h, specular_power);
                r += dl.color[0] * spec;
                g += dl.color[1] * spec;
                b += dl.color[2] * spec;
            }
        }

        // Point lights (closest MAX_POINT_LIGHTS only)
        let mut best_idx = [usize::MAX; MAX_POINT_LIGHTS];
        let mut best_dist_sq = [f32::INFINITY; MAX_POINT_LIGHTS];

        for (idx, pl) in self.point_lights.iter().enumerate() {
            let to_light = pl.position - world_pos;
            let dist_sq = to_light.length_sq();
            if dist_sq > pl.radius * pl.radius {
                continue;
            }

            let mut insert_at = MAX_POINT_LIGHTS;
            for slot in 0..MAX_POINT_LIGHTS {
                if dist_sq < best_dist_sq[slot] {
                    insert_at = slot;
                    break;
                }
            }

            if insert_at < MAX_POINT_LIGHTS {
                let mut slot = MAX_POINT_LIGHTS - 1;
                while slot > insert_at {
                    best_dist_sq[slot] = best_dist_sq[slot - 1];
                    best_idx[slot] = best_idx[slot - 1];
                    slot -= 1;
                }
                best_dist_sq[insert_at] = dist_sq;
                best_idx[insert_at] = idx;
            }
        }

        for slot in 0..MAX_POINT_LIGHTS {
            let idx = best_idx[slot];
            if idx == usize::MAX {
                continue;
            }

            let pl = &self.point_lights[idx];
            let to_light = pl.position - world_pos;
            let dist_sq = to_light.length_sq();
            if dist_sq <= 1e-12 {
                continue;
            }

            let inv_dist = crate::math::fast::inv_sqrt(dist_sq);
            let dist = dist_sq * inv_dist;
            let light_dir = to_light * inv_dist;

            let n_dot_l = normal.dot(light_dir);
            if n_dot_l <= 0.0 {
                continue;
            }

            // Smooth quadratic attenuation: 1 - (dist/radius)²
            // Carmack used this in Doom 3 — looks better than 1/d² (no harsh cutoff).
            let ratio = dist * pl.inv_radius;
            let atten = saturate(1.0 - ratio * ratio);
            let diff = saturate(n_dot_l) * atten;

            r += pl.color[0] * diff;
            g += pl.color[1] * diff;
            b += pl.color[2] * diff;

            if specular_power > 0.0 {
                let half = (light_dir + view_dir).normalize();
                let n_dot_h = saturate(normal.dot(half));
                let spec = pow_approx(n_dot_h, specular_power) * atten;
                r += pl.color[0] * spec;
                g += pl.color[1] * spec;
                b += pl.color[2] * spec;
            }
        }

        // Modulate by vertex color (baked AO, tinting)
        [
            r * vertex_color[0],
            g * vertex_color[1],
            b * vertex_color[2],
        ]
    }
}

/// Fast integer power approximation for specular highlights.
///
/// For N·H raised to a power (Phong exponent), we use the classic
/// squaring trick: pow(x, n) via repeated squaring in log2(n) steps.
///
/// For typical specular powers (8, 16, 32, 64), this is 3-6 multiplies
/// vs. the Taylor series (10+ multiplies) or a log-exp pair.
fn pow_approx(base: f32, exp: f32) -> f32 {
    if base <= 0.0 {
        return 0.0;
    }
    if exp <= 0.0 {
        return 1.0;
    }

    // Use 2^(exp * log2(base)) via fast_log2 and fast_exp2
    crate::math::fast::fast_exp2(exp * crate::math::fast::fast_log2(base))
}

/// Fog calculation.
///
/// Linear fog: factor = (end - z) / (end - start)
/// Exponential fog: factor = 2^(-density × z)
///
/// Returns fog factor in [0, 1] where 0 = fully fogged, 1 = no fog.
#[derive(Debug, Clone, Copy)]
pub enum FogMode {
    None,
    Linear { start: f32, end: f32 },
    Exponential { density: f32 },
}

impl FogMode {
    #[inline]
    pub fn compute(&self, distance: f32) -> f32 {
        match *self {
            FogMode::None => 1.0,
            FogMode::Linear { start, end } => {
                saturate((end - distance) * crate::math::fast::fast_recip(end - start))
            }
            FogMode::Exponential { density } => {
                saturate(crate::math::fast::fast_exp2(-density * distance))
            }
        }
    }
}
