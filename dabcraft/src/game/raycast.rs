use glam::{IVec3, Vec3};

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub block: IVec3,
    /// Unit outward normal of the struck face; `IVec3::ZERO` when the ray
    /// origin starts inside a solid block (no face was crossed).
    pub normal: IVec3,
    /// World-units along the ray to the face crossing.
    pub distance: f32,
}

/// Amanatides & Woo voxel DDA: visits every cell the ray passes through,
/// in order, until `max_dist` (entering a cell exactly at `max_dist` still
/// counts). `dir` need not be normalized; zero direction returns None.
#[cfg_attr(not(test), allow(dead_code))]
pub fn raycast(
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
    is_solid: &impl Fn(IVec3) -> bool,
) -> Option<RayHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let mut cell = [
        origin.x.floor() as i32,
        origin.y.floor() as i32,
        origin.z.floor() as i32,
    ];
    if is_solid(IVec3::from_array(cell)) {
        return Some(RayHit { block: IVec3::from_array(cell), normal: IVec3::ZERO, distance: 0.0 });
    }
    let o = [origin.x, origin.y, origin.z];
    let d = [dir.x, dir.y, dir.z];
    let mut step = [0i32; 3];
    let mut t_max = [f32::INFINITY; 3]; // ray length to the next boundary, per axis
    let mut t_delta = [f32::INFINITY; 3]; // ray length per whole cell, per axis
    for a in 0..3 {
        if d[a] > 0.0 {
            step[a] = 1;
            t_delta[a] = 1.0 / d[a];
            t_max[a] = ((cell[a] + 1) as f32 - o[a]) / d[a];
        } else if d[a] < 0.0 {
            step[a] = -1;
            t_delta[a] = -1.0 / d[a];
            t_max[a] = (o[a] - cell[a] as f32) / -d[a];
        }
    }
    loop {
        let axis = if t_max[0] <= t_max[1] && t_max[0] <= t_max[2] {
            0
        } else if t_max[1] <= t_max[2] {
            1
        } else {
            2
        };
        let distance = t_max[axis];
        if distance > max_dist {
            return None;
        }
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        if is_solid(IVec3::from_array(cell)) {
            let mut n = [0i32; 3];
            n[axis] = -step[axis];
            return Some(RayHit {
                block: IVec3::from_array(cell),
                normal: IVec3::from_array(n),
                distance,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{IVec3, Vec3};

    fn only(cell: IVec3) -> impl Fn(IVec3) -> bool {
        move |c: IVec3| c == cell
    }

    #[test]
    fn hits_block_along_positive_x() {
        let hit = raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::X, 6.0, &only(IVec3::new(3, 0, 0)))
            .expect("must hit");
        assert_eq!(hit.block, IVec3::new(3, 0, 0));
        assert_eq!(hit.normal, IVec3::new(-1, 0, 0), "entered through the -X face");
        assert!((hit.distance - 2.5).abs() < 1e-5);
    }

    #[test]
    fn hits_block_in_negative_coordinates() {
        let hit = raycast(
            Vec3::new(-0.5, 0.5, -0.5),
            Vec3::NEG_X,
            6.0,
            &only(IVec3::new(-4, 0, -1)),
        )
        .expect("must hit");
        assert_eq!(hit.block, IVec3::new(-4, 0, -1));
        assert_eq!(hit.normal, IVec3::new(1, 0, 0));
        assert!((hit.distance - 2.5).abs() < 1e-5, "from x=-0.5 to the x=-3 face");
    }

    #[test]
    fn vertical_ray_hits_underside() {
        let hit = raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::Y, 6.0, &only(IVec3::new(0, 5, 0)))
            .expect("must hit");
        assert_eq!(hit.normal, IVec3::new(0, -1, 0));
        assert!((hit.distance - 4.5).abs() < 1e-5);
    }

    #[test]
    fn respects_max_distance() {
        assert!(raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::X, 2.0, &only(IVec3::new(3, 0, 0))).is_none());
    }

    #[test]
    fn misses_when_nothing_is_solid() {
        assert!(raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::ONE, 6.0, &|_| false).is_none());
    }

    #[test]
    fn origin_inside_solid_reports_zero_distance_and_no_face() {
        let hit = raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::X, 6.0, &|_| true).expect("must hit");
        assert_eq!(hit.block, IVec3::new(0, 0, 0));
        assert_eq!(hit.normal, IVec3::ZERO);
        assert_eq!(hit.distance, 0.0);
    }

    #[test]
    fn diagonal_ray_walks_cells_in_order() {
        // From (0.5,0.5,0.5) along normalize(1,1,0): boundary ties resolve
        // x-first (the <= in the axis pick), so the visit order is
        // (0,0,0) (1,0,0) (1,1,0) (2,1,0) (2,2,0)…
        let hit = raycast(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(1.0, 1.0, 0.0).normalize(),
            10.0,
            &only(IVec3::new(2, 2, 0)),
        )
        .expect("must hit");
        assert_eq!(hit.block, IVec3::new(2, 2, 0));
        assert_eq!(hit.normal, IVec3::new(0, -1, 0), "y step entered it last");
    }

    #[test]
    fn zero_direction_returns_none() {
        assert!(raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::ZERO, 6.0, &|_| true).is_none());
    }
}
