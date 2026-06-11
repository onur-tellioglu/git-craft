use glam::{Mat4, Vec3};

pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,   // radians, 0 = -Z, positive = right
    pub pitch: f32, // radians, clamped
    pub fov_y: f32, // radians
}

impl Camera {
    pub const PITCH_LIMIT: f32 = 89.0 * std::f32::consts::PI / 180.0;
    const MOUSE_SENSITIVITY: f32 = 0.0022;

    pub fn new(position: Vec3) -> Self {
        Self { position, yaw: 0.0, pitch: 0.0, fov_y: 70f32.to_radians() }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }

    /// winit MouseMotion delta: +x = mouse right, +y = mouse down.
    pub fn apply_mouse_delta(&mut self, dx: f64, dy: f64) {
        self.yaw += dx as f32 * Self::MOUSE_SENSITIVITY;
        self.pitch = (self.pitch - dy as f32 * Self::MOUSE_SENSITIVITY)
            .clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    pub const FLY_SPEED: f32 = 20.0; // blocks per second
    pub const FAR_PLANE: f32 = 800.0;
    pub const SPRINT_MULTIPLIER: f32 = 8.0;

    pub fn fly(&mut self, input: &crate::game::input::InputState, dt: f32) {
        use winit::keyboard::KeyCode as K;
        let forward = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin());
        let mut dir = Vec3::ZERO;
        if input.is_down(K::KeyW) { dir += forward; }
        if input.is_down(K::KeyS) { dir -= forward; }
        if input.is_down(K::KeyD) { dir += right; }
        if input.is_down(K::KeyA) { dir -= right; }
        if input.is_down(K::Space) { dir += Vec3::Y; }
        if input.is_down(K::ShiftLeft) { dir -= Vec3::Y; }
        if dir != Vec3::ZERO {
            let speed = Self::FLY_SPEED * if input.is_down(K::ControlLeft) { Self::SPRINT_MULTIPLIER } else { 1.0 };
            self.position += dir.normalize() * speed * dt;
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let proj = Mat4::perspective_rh(self.fov_y, aspect, 0.1, Self::FAR_PLANE);
        let view = Mat4::look_to_rh(self.position, self.forward(), Vec3::Y);
        proj * view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Vec4};

    fn approx(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-5, "{a} != {b}");
    }

    #[test]
    fn default_orientation_looks_down_negative_z() {
        let cam = Camera::new(Vec3::ZERO);
        approx(cam.forward(), Vec3::NEG_Z);
    }

    #[test]
    fn positive_yaw_turns_right() {
        let mut cam = Camera::new(Vec3::ZERO);
        cam.yaw = std::f32::consts::FRAC_PI_2; // 90° right
        approx(cam.forward(), Vec3::X);
    }

    #[test]
    fn pitch_is_clamped() {
        let mut cam = Camera::new(Vec3::ZERO);
        cam.apply_mouse_delta(0.0, -10_000.0); // huge upward look
        assert!(cam.pitch <= Camera::PITCH_LIMIT);
        cam.apply_mouse_delta(0.0, 10_000.0);
        assert!(cam.pitch >= -Camera::PITCH_LIMIT);
    }

    #[test]
    fn fly_moves_horizontally_along_yaw_even_when_pitched() {
        let mut cam = Camera::new(Vec3::ZERO);
        cam.pitch = 1.0; // looking up
        let mut input = crate::game::input::InputState::default();
        input.set_key(winit::keyboard::KeyCode::KeyW, true);
        cam.fly(&input, 1.0);
        approx(cam.position, Vec3::new(0.0, 0.0, -Camera::FLY_SPEED)); // no vertical drift
    }

    #[test]
    fn space_and_shift_move_vertically() {
        let mut cam = Camera::new(Vec3::ZERO);
        let mut input = crate::game::input::InputState::default();
        input.set_key(winit::keyboard::KeyCode::Space, true);
        cam.fly(&input, 0.5);
        approx(cam.position, Vec3::new(0.0, Camera::FLY_SPEED * 0.5, 0.0));
    }

    #[test]
    fn opposing_keys_cancel() {
        let mut cam = Camera::new(Vec3::ZERO);
        let mut input = crate::game::input::InputState::default();
        input.set_key(winit::keyboard::KeyCode::KeyW, true);
        input.set_key(winit::keyboard::KeyCode::KeyS, true);
        cam.fly(&input, 1.0);
        approx(cam.position, Vec3::ZERO);
    }

    #[test]
    fn view_proj_maps_point_in_front_to_clip_space() {
        let cam = Camera::new(Vec3::ZERO);
        let vp = cam.view_proj(16.0 / 9.0);
        let clip = vp * Vec4::new(0.0, 0.0, -10.0, 1.0); // 10 units ahead
        let ndc = clip / clip.w;
        assert!(ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5, "centered point stays centered");
        assert!(ndc.z > 0.0 && ndc.z < 1.0, "wgpu depth range is 0..1, got {}", ndc.z);
    }

    #[test]
    fn far_plane_covers_render_distance_diagonal() {
        // 384 blocks horizontally + 256 world height: corner diagonal
        // sqrt(384² + 384² + 256²) ≈ 601. Far must comfortably exceed it.
        assert!(Camera::FAR_PLANE >= 700.0);
    }

    #[test]
    fn sprint_multiplies_speed() {
        let mut cam = Camera::new(Vec3::ZERO);
        let mut input = crate::game::input::InputState::default();
        input.set_key(winit::keyboard::KeyCode::KeyW, true);
        cam.fly(&input, 1.0);
        let normal = cam.position.length();
        let mut cam2 = Camera::new(Vec3::ZERO);
        input.set_key(winit::keyboard::KeyCode::ControlLeft, true);
        cam2.fly(&input, 1.0);
        assert!((cam2.position.length() - normal * Camera::SPRINT_MULTIPLIER).abs() < 1e-3);
    }
}
