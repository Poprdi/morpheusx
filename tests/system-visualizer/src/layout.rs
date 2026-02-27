use morpheus_gfx3d::math::vec::Vec3;
use crate::state::SystemState;

const MAX_PROCS: usize = 64;
const REPULSION_ITERS: usize = 3;
const MIN_SEP: f32 = 1.8;

// Kernel (PID 0) is positioned at the galaxy location for thematic unity
const KERNEL_GALAXY_POS: Vec3 = Vec3 { x: 0.0, y: -15.0, z: -35.0 };

pub struct ProcessLayout {
    pub positions: [Vec3; MAX_PROCS],
    pub radii: [f32; MAX_PROCS],
    pub count: usize,
    smooth: [Vec3; MAX_PROCS],
}

impl ProcessLayout {
    pub fn new() -> Self {
        Self {
            positions: [Vec3::ZERO; MAX_PROCS],
            radii: [0.0; MAX_PROCS],
            count: 0,
            smooth: [Vec3::ZERO; MAX_PROCS],
        }
    }

    pub fn update(&mut self, state: &SystemState, dt: f32) {
        let n = state.proc_count;
        self.count = n;

        if n == 0 { return; }

        let children = build_child_map(state);
        let root = find_root(state);

        let mut depth = [0u32; MAX_PROCS];
        let mut order = [0usize; MAX_PROCS];
        let mut order_len = 0usize;

        let mut queue = [0usize; MAX_PROCS];
        let mut qh = 0usize;
        let mut qt = 0usize;

        if let Some(ri) = root {
            queue[qt] = ri;
            qt += 1;
            depth[ri] = 0;
        }

        while qh < qt {
            let idx = queue[qh];
            qh += 1;
            order[order_len] = idx;
            order_len += 1;

            let d = depth[idx] + 1;
            for ci in 0..children.counts[idx] as usize {
                let child = children.children[idx][ci] as usize;
                if child < n {
                    depth[child] = d;
                    if qt < MAX_PROCS {
                        queue[qt] = child;
                        qt += 1;
                    }
                }
            }
        }

        for i in 0..n {
            if !order[..order_len].contains(&i) {
                if order_len < MAX_PROCS {
                    order[order_len] = i;
                    order_len += 1;
                    depth[i] = 1;
                }
            }
        }

        let mut level_count = [0u32; 16];
        let mut level_idx = [0u32; MAX_PROCS];

        for i in 0..n {
            // Kernel lives at the galaxy position; exclude from ring layout counts
            if let Some(p) = state.process(i) { if p.pid == 0 { continue; } }
            let d = (depth[i] as usize).min(15);
            level_idx[i] = level_count[d];
            level_count[d] += 1;
        }

        let two_pi = 2.0 * core::f32::consts::PI;

        for i in 0..n {
            let proc = match state.process(i) {
                Some(p) => p,
                None => continue,
            };
            
            // Kernel (PID 0) is positioned at the galaxy center
            if proc.pid == 0 {
                self.positions[i] = KERNEL_GALAXY_POS;
                // Kernel gets a generous radius to make it prominent
                self.radii[i] = 1.2;
                continue;
            }
            
            let d = depth[i] as usize;
            let siblings = level_count[d.min(15)].max(1) as f32;
            let min_ring = siblings * MIN_SEP / two_pi;
            let base_ring = 3.5 + (d as f32 - 1.0) * 4.0;
            let ring_radius = if d == 0 { 0.0 } else if base_ring < min_ring { min_ring } else { base_ring };
            let idx_at_depth = level_idx[i] as f32;

            let angle = if siblings > 0.0 {
                (idx_at_depth / siblings) * two_pi + 0.3
            } else {
                0.0
            };

            // --- 3-D scatter ---
            // Base Y is deeper per depth level (was 1.5, now 2.5 for more
            // vertical separation between hierarchy levels).
            // Within a ring, offset Y sinusoidally so same-depth siblings
            // form a loose helix rather than a flat disk.
            // Alternate the ring radius slightly so adjacent siblings aren't
            // all at the exact same distance from center.
            let depth_y      = -(d as f32) * 2.5;
            let helix_y      = fast_sin(angle * 1.7) * 1.2;
            let radius_nudge = if (level_idx[i] & 1) == 0 { 0.0 } else { 0.5 };
            let x = (ring_radius + radius_nudge) * fast_sin(angle);
            let z = (ring_radius + radius_nudge) * fast_cos(angle);
            let y = depth_y + helix_y;

            self.positions[i] = Vec3::new(x, y, z);
            let mem_scale = fast_ln(proc.pages_alloc.max(1) as f32) * 0.15;
            self.radii[i] = 0.25 + clamp(mem_scale, 0.0, 0.75);
        }

        for _ in 0..REPULSION_ITERS {
            for i in 0..n {
                // Skip kernel entry at galaxy position — its radius would
                // create phantom repulsion far from the process cloud.
                if let Some(p) = state.process(i) { if p.pid == 0 { continue; } }
                for j in (i + 1)..n {
                    let a = self.positions[i];
                    let b = self.positions[j];
                    let dx = b.x - a.x;
                    let dy = b.y - a.y;
                    let dz = b.z - a.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    let min_d = self.radii[i] + self.radii[j] + 0.6;
                    let min_sq = min_d * min_d;
                    if dist_sq < min_sq && dist_sq > 0.0001 {
                        let dist = fast_sqrt(dist_sq);
                        let overlap = (min_d - dist) * 0.5;
                        let inv = overlap / dist;
                        self.positions[i].x -= dx * inv;
                        self.positions[i].z -= dz * inv;
                        self.positions[j].x += dx * inv;
                        self.positions[j].z += dz * inv;
                    }
                }
            }
        }

        let lerp_rate = clamp(1.0 - fast_exp(-6.0 * dt), 0.0, 1.0);
        for i in 0..n {
            self.smooth[i] = vec3_lerp(self.smooth[i], self.positions[i], lerp_rate);
        }
    }

    pub fn smoothed(&self, idx: usize) -> Vec3 {
        if idx < self.count { self.smooth[idx] } else { Vec3::ZERO }
    }
}

struct ChildMap {
    children: [[u8; 8]; MAX_PROCS],
    counts: [u8; MAX_PROCS],
}

fn build_child_map(state: &SystemState) -> ChildMap {
    let mut map = ChildMap {
        children: [[0; 8]; MAX_PROCS],
        counts: [0; MAX_PROCS],
    };
    let procs = state.processes();
    for (i, p) in procs.iter().enumerate() {
        if let Some(pi) = state.find_index_by_pid(p.ppid) {
            if pi != i {
                let c = map.counts[pi] as usize;
                if c < 8 {
                    map.children[pi][c] = i as u8;
                    map.counts[pi] += 1;
                }
            }
        }
    }
    map
}

fn find_root(state: &SystemState) -> Option<usize> {
    let procs = state.processes();
    procs.iter().position(|p| p.pid == 1)
        .or_else(|| {
            if procs.is_empty() { None }
            else { Some(procs.iter().enumerate().min_by_key(|(_, p)| p.pid).map(|(i, _)| i).unwrap_or(0)) }
        })
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

fn fast_sqrt(x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }
    let i = f32::to_bits(x);
    let i = (i >> 1) + 0x1FC00000;
    let y = f32::from_bits(i);
    0.5 * (y + x / y)
}

fn vec3_lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    Vec3::new(
        a.x + (b.x - a.x) * t,
        a.y + (b.y - a.y) * t,
        a.z + (b.z - a.z) * t,
    )
}

fn fast_ln(x: f32) -> f32 {
    let bits = f32::to_bits(x);
    let exp = ((bits >> 23) & 0xFF) as i32 - 127;
    let mantissa = f32::from_bits((bits & 0x007FFFFF) | 0x3F800000);
    let m = mantissa - 1.0;
    (exp as f32) * 0.6931472 + m * (1.0 - m * 0.5)
}

fn fast_exp(x: f32) -> f32 {
    if x > 20.0 { return f32::MAX; }
    if x < -20.0 { return 0.0; }
    let t = 1.0 + x / 256.0;
    let mut r = t;
    for _ in 0..8 { r = r * r; }
    r
}

fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo { lo } else if v > hi { hi } else { v }
}
