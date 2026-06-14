use glam::{IVec3, Vec3};
use winit::keyboard::KeyCode as K;

use crate::game::input::InputState;
use crate::game::physics::{Aabb, move_aabb};

pub const WIDTH: f32 = 0.6;
pub const HEIGHT: f32 = 1.8;
pub const EYE_HEIGHT: f32 = 1.62;
pub const WALK_SPEED: f32 = 4.3; // blocks per second
pub const SPRINT_MULTIPLIER: f32 = 1.6;
pub const FLY_SPEED: f32 = 20.0; // M2's free-flight values, unchanged
pub const FLY_SPRINT_MULTIPLIER: f32 = 8.0;
pub const WATER_SPEED_FACTOR: f32 = 0.5;
pub const WATER_SINK_SPEED: f32 = 3.0;
const GRAVITY: f32 = 32.0;
// Continuous peak v²/2g ≈ 1.32 blocks, plus a discrete bonus of up to v·dt/2
// (jump frame moves at full speed before gravity bites): ~1.4–1.6 in practice.
const JUMP_SPEED: f32 = 9.2;
const TERMINAL_FALL: f32 = 78.0;
const SWIM_UP_SPEED: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveMode {
    Walk,
    Fly,
}

pub struct Player {
    /// Feet center (AABB bottom-center).
    pub position: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub mode: MoveMode,
}

impl Player {
    /// Starts in Fly: the world streams in around the spawn point exactly
    /// like M2; the player toggles to Walk when there is ground to walk on.
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            on_ground: false,
            mode: MoveMode::Fly,
        }
    }

    pub fn aabb(&self) -> Aabb {
        Aabb::from_feet(self.position, WIDTH, HEIGHT)
    }

    pub fn eye(&self) -> Vec3 {
        self.position + Vec3::new(0.0, EYE_HEIGHT, 0.0)
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            MoveMode::Walk => MoveMode::Fly,
            MoveMode::Fly => MoveMode::Walk,
        };
        self.velocity = Vec3::ZERO;
        self.on_ground = false;
    }

    /// One physics step. `is_solid` is the collision query (water and air
    /// are passable; unloaded terrain is the caller's choice — App passes
    /// solid). `is_water` drives swim movement.
    pub fn update(
        &mut self,
        input: &InputState,
        yaw: f32,
        dt: f32,
        is_solid: &impl Fn(IVec3) -> bool,
        is_water: &impl Fn(IVec3) -> bool,
    ) {
        match self.mode {
            MoveMode::Fly => self.update_fly(input, yaw, dt),
            MoveMode::Walk => self.update_walk(input, yaw, dt, is_solid, is_water),
        }
    }

    /// WASD direction on the ground plane from yaw (unnormalized sum).
    fn wish_dir(input: &InputState, yaw: f32) -> Vec3 {
        let forward = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        let right = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let mut dir = Vec3::ZERO;
        if input.is_down(K::KeyW) {
            dir += forward;
        }
        if input.is_down(K::KeyS) {
            dir -= forward;
        }
        if input.is_down(K::KeyD) {
            dir += right;
        }
        if input.is_down(K::KeyA) {
            dir -= right;
        }
        dir
    }

    /// M2 free flight, verbatim semantics: no gravity, no collision.
    fn update_fly(&mut self, input: &InputState, yaw: f32, dt: f32) {
        let mut dir = Self::wish_dir(input, yaw);
        if input.is_down(K::Space) {
            dir += Vec3::Y;
        }
        if input.is_down(K::ShiftLeft) {
            dir -= Vec3::Y;
        }
        if dir != Vec3::ZERO {
            let speed = FLY_SPEED
                * if input.is_down(K::ControlLeft) {
                    FLY_SPRINT_MULTIPLIER
                } else {
                    1.0
                };
            self.position += dir.normalize() * speed * dt;
        }
    }

    fn update_walk(
        &mut self,
        input: &InputState,
        yaw: f32,
        dt: f32,
        is_solid: &impl Fn(IVec3) -> bool,
        is_water: &impl Fn(IVec3) -> bool,
    ) {
        let feet_cell = self.position.floor().as_ivec3();
        let in_water = is_water(feet_cell) || is_water(self.eye().floor().as_ivec3());

        let wish = Self::wish_dir(input, yaw).normalize_or_zero();
        let mut speed = WALK_SPEED
            * if input.is_down(K::ControlLeft) {
                SPRINT_MULTIPLIER
            } else {
                1.0
            };
        if in_water {
            speed *= WATER_SPEED_FACTOR;
        }
        self.velocity.x = wish.x * speed;
        self.velocity.z = wish.z * speed;

        if in_water {
            // Spec §7: slow sinking, hold-jump to swim upward.
            self.velocity.y = if input.is_down(K::Space) {
                SWIM_UP_SPEED
            } else {
                (self.velocity.y - GRAVITY * 0.25 * dt).max(-WATER_SINK_SPEED)
            };
        } else {
            // Gravity first, jump second: the jump frame leaves with the full
            // JUMP_SPEED displacement, so jump height cannot dip below one
            // block at low frame rates (large dt).
            self.velocity.y = (self.velocity.y - GRAVITY * dt).max(-TERMINAL_FALL);
            // on_ground is last frame's contact: on the ledge walk-off frame a
            // jump still fires (one frame of coyote time, as in Minecraft).
            if self.on_ground && input.is_down(K::Space) {
                self.velocity.y = JUMP_SPEED;
            }
        }

        let (moved, hit) = move_aabb(self.aabb(), self.velocity * dt, is_solid);
        self.position = Vec3::new(
            (moved.min.x + moved.max.x) * 0.5,
            moved.min.y,
            (moved.min.z + moved.max.z) * 0.5,
        );
        // Read velocity.y BEFORE the zeroing below: its sign separates floor
        // hits (negative → grounded) from ceiling hits (positive → airborne).
        self.on_ground = hit[1] && self.velocity.y <= 0.0;
        if hit[0] {
            self.velocity.x = 0.0;
        }
        if hit[1] {
            self.velocity.y = 0.0;
        }
        if hit[2] {
            self.velocity.z = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{IVec3, Vec3};
    use winit::keyboard::KeyCode as K;

    const DT: f32 = 0.05;

    fn no_blocks(_: IVec3) -> bool {
        false
    }

    fn floor_at(y: i32) -> impl Fn(IVec3) -> bool {
        move |c: IVec3| c.y == y
    }

    fn keys(down: &[K]) -> crate::game::input::InputState {
        let mut input = crate::game::input::InputState::default();
        for &k in down {
            input.set_key(k, true);
        }
        input
    }

    fn walking_at(pos: Vec3) -> Player {
        let mut p = Player::new(pos);
        p.mode = MoveMode::Walk;
        p
    }

    #[test]
    fn spawns_in_fly_mode() {
        assert_eq!(Player::new(Vec3::ZERO).mode, MoveMode::Fly);
    }

    #[test]
    fn eye_sits_at_eye_height_above_feet() {
        let p = Player::new(Vec3::new(1.0, 64.0, 2.0));
        assert_eq!(p.eye(), Vec3::new(1.0, 64.0 + EYE_HEIGHT, 2.0));
    }

    #[test]
    fn walk_falls_under_gravity_and_lands() {
        let mut p = walking_at(Vec3::new(0.5, 66.0, 0.5));
        let input = keys(&[]);
        for _ in 0..60 {
            p.update(&input, 0.0, DT, &floor_at(63), &|_| false);
        }
        assert!(p.on_ground);
        assert!((p.position.y - 64.0).abs() < 1e-2, "rests on top of y=63");
        assert_eq!(p.velocity.y, 0.0);
    }

    #[test]
    fn jump_fires_only_when_grounded() {
        let mut p = walking_at(Vec3::new(0.5, 64.0 + 1e-4, 0.5));
        let input = keys(&[K::Space]);
        p.update(&input, 0.0, DT, &floor_at(63), &|_| false); // grounds first
        p.update(&input, 0.0, DT, &floor_at(63), &|_| false); // then jumps
        let mut peak = p.position.y;
        for _ in 0..40 {
            p.update(&keys(&[]), 0.0, DT, &floor_at(63), &|_| false);
            peak = peak.max(p.position.y);
        }
        assert!(peak > 65.0, "jump cleared one block, peak {peak}");
        assert!(peak < 65.6, "jump stays under 1.6 blocks, peak {peak}");
    }

    #[test]
    fn airborne_space_does_not_jump() {
        let mut p = walking_at(Vec3::new(0.5, 80.0, 0.5));
        let v0 = p.velocity.y;
        p.update(&keys(&[K::Space]), 0.0, DT, &no_blocks, &|_| false);
        assert!(p.velocity.y < v0, "still accelerating downward");
    }

    #[test]
    fn walks_forward_at_walk_speed() {
        let mut p = walking_at(Vec3::new(0.5, 64.0 + 1e-4, 0.5));
        p.on_ground = true;
        p.update(&keys(&[K::KeyW]), 0.0, DT, &floor_at(63), &|_| false);
        // yaw 0 looks down -Z.
        assert!((p.position.z - (0.5 - WALK_SPEED * DT)).abs() < 1e-4);
        assert_eq!(p.position.x, 0.5);
    }

    #[test]
    fn fly_sprint_multiplies_speed() {
        let mut p = Player::new(Vec3::new(0.5, 64.0, 0.5));
        p.update(
            &keys(&[K::KeyW, K::ControlLeft]),
            0.0,
            1.0,
            &no_blocks,
            &|_| false,
        );
        let expected = 0.5 - FLY_SPEED * FLY_SPRINT_MULTIPLIER;
        assert!((p.position.z - expected).abs() < 1e-3);
    }

    #[test]
    fn sprint_scales_walk_speed() {
        let mut p = walking_at(Vec3::new(0.5, 64.0 + 1e-4, 0.5));
        p.update(
            &keys(&[K::KeyW, K::ControlLeft]),
            0.0,
            DT,
            &floor_at(63),
            &|_| false,
        );
        assert!((p.position.z - (0.5 - WALK_SPEED * SPRINT_MULTIPLIER * DT)).abs() < 1e-4);
    }

    #[test]
    fn fly_ignores_gravity_and_blocks() {
        let mut p = Player::new(Vec3::new(0.5, 64.0, 0.5));
        p.update(&keys(&[]), 0.0, DT, &|_| true, &|_| false);
        assert_eq!(p.position, Vec3::new(0.5, 64.0, 0.5), "no input, no motion");
        p.update(&keys(&[K::KeyW]), 0.0, 1.0, &|_| true, &|_| false);
        assert!(
            (p.position.z - (0.5 - FLY_SPEED)).abs() < 1e-4,
            "moves through solids"
        );
    }

    #[test]
    fn toggle_swaps_mode_and_zeroes_velocity() {
        let mut p = walking_at(Vec3::ZERO);
        p.velocity = Vec3::new(1.0, -5.0, 0.0);
        p.toggle_mode();
        assert_eq!(p.mode, MoveMode::Fly);
        assert_eq!(p.velocity, Vec3::ZERO);
        p.toggle_mode();
        assert_eq!(p.mode, MoveMode::Walk);
    }

    #[test]
    fn water_sinks_slowly_and_swims_up() {
        let everywhere_water = |_: IVec3| true;
        let mut p = walking_at(Vec3::new(0.5, 70.0, 0.5));
        for _ in 0..40 {
            p.update(&keys(&[]), 0.0, DT, &no_blocks, &everywhere_water);
        }
        assert!(
            p.velocity.y >= -WATER_SINK_SPEED - 1e-4,
            "sink speed is capped"
        );
        let y_before = p.position.y;
        for _ in 0..10 {
            p.update(&keys(&[K::Space]), 0.0, DT, &no_blocks, &everywhere_water);
        }
        assert!(p.position.y > y_before, "holding space swims upward");
    }

    #[test]
    fn water_halves_walk_speed() {
        let everywhere_water = |_: IVec3| true;
        let mut p = walking_at(Vec3::new(0.5, 70.0, 0.5));
        p.update(&keys(&[K::KeyW]), 0.0, DT, &no_blocks, &everywhere_water);
        let dz = (0.5 - p.position.z) / DT;
        assert!((dz - WALK_SPEED * WATER_SPEED_FACTOR).abs() < 1e-3);
    }
}
