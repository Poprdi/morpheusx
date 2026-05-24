use crate::math::mat4::Mat4;
use crate::math::trig::TrigTable;
use crate::math::vec::Vec3;

/// FPS camera, yaw/pitch only (no roll).
pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,   // radians, 0 = looking along -Z
    pub pitch: f32, // radians, clamped to ±89°
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

const MAX_PITCH: f32 = 1.5533; // ~89° — leaves headroom from gimbal lock

impl Camera {
    pub fn new(aspect: f32) -> Self {
        Self {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: core::f32::consts::FRAC_PI_3, // 60°
            aspect,
            near: 0.1,
            far: 1000.0,
        }
    }

    /// Deltas in radians (typical mouse sensitivity ~0.002 rad/count).
    pub fn rotate(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw += delta_yaw;
        self.pitch += delta_pitch;

        let two_pi = 2.0 * core::f32::consts::PI;
        while self.yaw < 0.0 {
            self.yaw += two_pi;
        }
        while self.yaw >= two_pi {
            self.yaw -= two_pi;
        }

        if self.pitch > MAX_PITCH {
            self.pitch = MAX_PITCH;
        }
        if self.pitch < -MAX_PITCH {
            self.pitch = -MAX_PITCH;
        }
    }

    /// Movement ignores pitch (standard FPS walk-along-ground).
    pub fn translate(&mut self, forward: f32, right: f32, up: f32, trig: &TrigTable) {
        let (sin_yaw, cos_yaw) = trig.sin_cos(self.yaw);

        let fwd = Vec3::new(-sin_yaw, 0.0, -cos_yaw);
        let rgt = Vec3::new(cos_yaw, 0.0, -sin_yaw);

        self.position += fwd * forward;
        self.position += rgt * right;
        self.position += Vec3::UP * up;
    }

    pub fn forward(&self, trig: &TrigTable) -> Vec3 {
        let (sin_yaw, cos_yaw) = trig.sin_cos(self.yaw);
        let (sin_pitch, cos_pitch) = trig.sin_cos(self.pitch);
        Vec3::new(-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch)
    }

    pub fn view_matrix(&self, trig: &TrigTable) -> Mat4 {
        let fwd = self.forward(trig);
        let target = self.position + fwd;
        Mat4::look_at(self.position, target, Vec3::UP)
    }

    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::perspective(self.fov_y, self.aspect, self.near, self.far)
    }

    pub fn view_proj(&self, trig: &TrigTable) -> Mat4 {
        let view = self.view_matrix(trig);
        let proj = self.projection_matrix();
        proj.mul(&view)
    }
}
