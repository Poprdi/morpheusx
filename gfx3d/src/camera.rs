use crate::math::mat4::Mat4;
use crate::math::trig::TrigTable;
use crate::math::vec::Vec3;

/// FPS-style camera with Euler angles (yaw/pitch).
///
/// No roll — roll makes players nauseous in FPS games and adds complexity
/// for zero benefit. Quake, Doom, Half-Life — none of them use camera roll.
pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,    // radians, 0 = looking along -Z
    pub pitch: f32,  // radians, clamped to ±89°
    pub fov_y: f32,  // vertical field of view in radians
    pub aspect: f32, // width / height
    pub near: f32,
    pub far: f32,
}

const MAX_PITCH: f32 = 1.5533; // ~89 degrees in radians

impl Camera {
    pub fn new(aspect: f32) -> Self {
        Self {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: core::f32::consts::FRAC_PI_3, // 60 degrees
            aspect,
            near: 0.1,
            far: 1000.0,
        }
    }

    /// Rotate camera from mouse delta.
    ///
    /// Sensitivity: radians per mouse unit. Typical: 0.002 for raw PS/2 counts.
    pub fn rotate(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw += delta_yaw;
        self.pitch += delta_pitch;

        // Wrap yaw to [0, 2π)
        let two_pi = 2.0 * core::f32::consts::PI;
        while self.yaw < 0.0 {
            self.yaw += two_pi;
        }
        while self.yaw >= two_pi {
            self.yaw -= two_pi;
        }

        // Clamp pitch to prevent gimbal lock
        if self.pitch > MAX_PITCH {
            self.pitch = MAX_PITCH;
        }
        if self.pitch < -MAX_PITCH {
            self.pitch = -MAX_PITCH;
        }
    }

    /// Move camera relative to its current facing direction.
    ///
    /// `forward`/`right`/`up` are in camera-local space.
    /// Uses TrigTable for sin/cos to avoid FPU sin/cos.
    pub fn translate(&mut self, forward: f32, right: f32, up: f32, trig: &TrigTable) {
        let (sin_yaw, cos_yaw) = trig.sin_cos(self.yaw);

        // Forward direction in world space (ignoring pitch for movement — standard FPS)
        let fwd = Vec3::new(-sin_yaw, 0.0, -cos_yaw);
        let rgt = Vec3::new(cos_yaw, 0.0, -sin_yaw);

        self.position += fwd * forward;
        self.position += rgt * right;
        self.position += Vec3::UP * up;
    }

    /// Compute forward direction vector.
    pub fn forward(&self, trig: &TrigTable) -> Vec3 {
        let (sin_yaw, cos_yaw) = trig.sin_cos(self.yaw);
        let (sin_pitch, cos_pitch) = trig.sin_cos(self.pitch);
        Vec3::new(-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch)
    }

    /// Build the view matrix.
    pub fn view_matrix(&self, trig: &TrigTable) -> Mat4 {
        let fwd = self.forward(trig);
        let target = self.position + fwd;
        Mat4::look_at(self.position, target, Vec3::UP)
    }

    /// Build the projection matrix.
    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::perspective(self.fov_y, self.aspect, self.near, self.far)
    }

    /// Combined view × projection (used for frustum extraction and vertex transform).
    pub fn view_proj(&self, trig: &TrigTable) -> Mat4 {
        let view = self.view_matrix(trig);
        let proj = self.projection_matrix();
        proj.mul(&view)
    }
}
