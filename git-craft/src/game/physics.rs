use glam::{IVec3, Vec3};

/// Collision skin: resolved positions stay this far off voxel faces so
/// float equality at boundaries never re-collides on the next frame.
const SKIN: f32 = 1e-4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// Box from a feet-center position (player convention: position is the
    /// AABB bottom-center).
    pub fn from_feet(feet: Vec3, width: f32, height: f32) -> Self {
        let half = width * 0.5;
        Self {
            min: Vec3::new(feet.x - half, feet.y, feet.z - half),
            max: Vec3::new(feet.x + half, feet.y + height, feet.z + half),
        }
    }

    pub fn translated(self, t: Vec3) -> Self {
        Self { min: self.min + t, max: self.max + t }
    }

    /// Does this box overlap the unit voxel at `cell`? (Strict inequality:
    /// exact face contact is not overlap.)
    pub fn intersects_cell(&self, cell: IVec3) -> bool {
        let lo = cell.as_vec3();
        let hi = lo + Vec3::ONE;
        self.min.x < hi.x
            && self.max.x > lo.x
            && self.min.y < hi.y
            && self.max.y > lo.y
            && self.min.z < hi.z
            && self.max.z > lo.z
    }
}

fn make_cell(axis: usize, p: i32, u: usize, i: i32, v: usize, j: i32) -> IVec3 {
    let mut c = [0i32; 3];
    c[axis] = p;
    c[u] = i;
    c[v] = j;
    IVec3::from_array(c)
}

/// Clamp a single-axis move against solid voxels: scan voxel planes from
/// the leading face outward to the move's end. Returns the allowed signed
/// distance (|allowed| ≤ |delta|). Plane scanning (not stepping) means
/// arbitrarily high velocity cannot tunnel.
fn sweep_axis(b: Aabb, axis: usize, delta: f32, is_solid: &impl Fn(IVec3) -> bool) -> f32 {
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    // Cross-section footprint, shrunk by SKIN so faces in exact contact on
    // the perpendicular axes don't count as overlap.
    let u0 = (b.min[u] + SKIN).floor() as i32;
    let u1 = (b.max[u] - SKIN).floor() as i32;
    let v0 = (b.min[v] + SKIN).floor() as i32;
    let v1 = (b.max[v] - SKIN).floor() as i32;
    let blocked =
        |p: i32| (u0..=u1).any(|i| (v0..=v1).any(|j| is_solid(make_cell(axis, p, u, i, v, j))));

    if delta > 0.0 {
        let lead = b.max[axis];
        let first = (lead + SKIN).floor() as i32;
        let last = (lead + delta).floor() as i32;
        for p in first..=last {
            if blocked(p) {
                return (p as f32 - lead - SKIN).clamp(0.0, delta);
            }
        }
        delta
    } else {
        let lead = b.min[axis];
        let first = (lead - SKIN).floor() as i32;
        let last = (lead + delta).floor() as i32;
        for p in (last..=first).rev() {
            if blocked(p) {
                return ((p + 1) as f32 - lead + SKIN).clamp(delta, 0.0);
            }
        }
        delta
    }
}

/// Move `aabb` by `delta` with axis-separated swept collision, Y first
/// (grounding must resolve before horizontal sliding), then X, then Z.
/// Returns the moved box and per-axis hit flags `[x, y, z]`.
pub fn move_aabb(
    aabb: Aabb,
    delta: Vec3,
    is_solid: &impl Fn(IVec3) -> bool,
) -> (Aabb, [bool; 3]) {
    let mut b = aabb;
    let mut hit = [false; 3];
    for axis in [1usize, 0, 2] {
        let d = delta[axis];
        if d == 0.0 {
            continue;
        }
        let allowed = sweep_axis(b, axis, d, is_solid);
        hit[axis] = allowed != d;
        let mut t = [0.0f32; 3];
        t[axis] = allowed;
        b = b.translated(Vec3::from_array(t));
    }
    (b, hit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{IVec3, Vec3};

    const W: f32 = 0.6;
    const H: f32 = 1.8;

    fn no_blocks(_: IVec3) -> bool {
        false
    }

    /// Infinite flat floor: every cell at y == `y` is solid.
    fn floor_at(y: i32) -> impl Fn(IVec3) -> bool {
        move |c: IVec3| c.y == y
    }

    /// Infinite wall: every cell at x == `x` is solid.
    fn wall_at(x: i32) -> impl Fn(IVec3) -> bool {
        move |c: IVec3| c.x == x
    }

    #[test]
    fn from_feet_builds_centered_box() {
        let b = Aabb::from_feet(Vec3::new(10.0, 64.0, -3.0), W, H);
        assert_eq!(b.min, Vec3::new(9.7, 64.0, -3.3));
        assert_eq!(b.max, Vec3::new(10.3, 65.8, -2.7));
    }

    #[test]
    fn intersects_cell_checks_unit_voxel_overlap() {
        let b = Aabb::from_feet(Vec3::new(0.5, 64.0, 0.5), W, H);
        assert!(b.intersects_cell(IVec3::new(0, 64, 0)));
        assert!(b.intersects_cell(IVec3::new(0, 65, 0)), "1.8 tall spans two cells");
        assert!(!b.intersects_cell(IVec3::new(0, 66, 0)), "head ends at 65.8");
        assert!(!b.intersects_cell(IVec3::new(2, 64, 0)));
    }

    #[test]
    fn falls_freely_without_blocks() {
        let b = Aabb::from_feet(Vec3::new(0.5, 100.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, -10.0, 0.0), &no_blocks);
        assert!((moved.min.y - 90.0).abs() < 1e-4);
        assert_eq!(hit, [false; 3]);
    }

    #[test]
    fn lands_on_floor_even_at_high_velocity() {
        // Spec §9: high-velocity edge case — a 200-block fall in one step
        // must clamp at the floor, not tunnel through it.
        let b = Aabb::from_feet(Vec3::new(0.5, 100.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, -200.0, 0.0), &floor_at(63));
        assert!((moved.min.y - 64.0).abs() < 1e-3, "rests on top of y=63 cells");
        assert!(hit[1]);
        assert!(!hit[0] && !hit[2]);
    }

    #[test]
    fn slides_along_wall_without_snagging() {
        // Axis separation: x is clamped by the wall, z moves the full
        // distance — the classic no-corner-snag behavior.
        let b = Aabb::from_feet(Vec3::new(4.5, 0.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(1.0, 0.0, 2.0), &wall_at(5));
        assert!((moved.max.x - 5.0).abs() < 1e-3, "stopped at the wall plane");
        assert!(hit[0]);
        assert!((moved.min.z - 2.2).abs() < 1e-4, "z slid the full 2.0");
        assert!(!hit[2]);
    }

    #[test]
    fn exact_touch_is_not_a_collision() {
        // Spec §9: exact-touch edge case. Box face exactly on the wall
        // plane, moving parallel to it: full move, no collision flag.
        let b = Aabb::from_feet(Vec3::new(4.7, 0.0, 0.5), W, H); // max.x == 5.0
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, 0.0, 1.0), &wall_at(5));
        assert_eq!(hit, [false; 3]);
        assert!((moved.min.z - 1.2).abs() < 1e-4);
    }

    #[test]
    fn pushing_into_a_touching_wall_moves_zero_not_negative() {
        let start = 5.0 - 0.3 - 1e-4; // resting against the wall with skin
        let b = Aabb::from_feet(Vec3::new(start, 0.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.5, 0.0, 0.0), &wall_at(5));
        assert!(hit[0]);
        assert!((moved.min.x - b.min.x).abs() < 1e-3, "no backward ejection");
    }

    #[test]
    fn ceiling_stops_upward_motion() {
        let b = Aabb::from_feet(Vec3::new(0.5, 64.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, 5.0, 0.0), &floor_at(67));
        assert!((moved.max.y - 67.0).abs() < 1e-3, "head clamped under y=67 cells");
        assert!(hit[1]);
    }

    #[test]
    fn corner_block_does_not_snag_parallel_motion() {
        // Spec §9: corner edge case. A single block beside the path must
        // not stop motion that merely grazes it.
        let solid = |c: IVec3| c == IVec3::new(1, 0, 2);
        let b = Aabb::from_feet(Vec3::new(0.5, 0.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, 0.0, 3.0), &solid);
        // Box spans x 0.2..0.8 — clear of cell x=1; it passes the corner.
        assert_eq!(hit, [false; 3]);
        assert!((moved.min.z - 3.2).abs() < 1e-4);
    }
}
