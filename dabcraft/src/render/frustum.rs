use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};

/// View frustum as 6 inward-facing planes (xyz = normal, w = distance):
/// dot(n, p) + w >= 0 ⇔ p on the visible side.
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Gribb–Hartmann extraction. wgpu clip space: x,y ∈ -1..1, z ∈ 0..1,
    /// so near = row2 and far = row3 - row2 (NOT the GL row3±row2 pair).
    pub fn from_view_proj(m: Mat4) -> Self {
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        let mut planes = [
            r3 + r0, // left
            r3 - r0, // right
            r3 + r1, // bottom
            r3 - r1, // top
            r2,      // near (z >= 0)
            r3 - r2, // far  (z <= w)
        ];
        for p in &mut planes {
            let len = p.xyz().length();
            debug_assert!(len > 1e-6, "degenerate view-projection matrix");
            *p /= len;
        }
        Self { planes }
    }

    /// Positive-vertex test: for each plane, check the AABB corner farthest
    /// along the plane normal; if even that corner is outside, the whole box
    /// is. Conservative (a box outside only a frustum *edge* can pass),
    /// which is correct for culling — never discards visible geometry.
    pub fn intersects_aabb(&self, min: Vec3, max: Vec3) -> bool {
        for p in &self.planes {
            let positive = Vec3::new(
                if p.x >= 0.0 { max.x } else { min.x },
                if p.y >= 0.0 { max.y } else { min.y },
                if p.z >= 0.0 { max.z } else { min.z },
            );
            if p.xyz().dot(positive) + p.w < 0.0 {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};

    fn camera_at_origin_looking_minus_z() -> Frustum {
        let proj = Mat4::perspective_rh(70f32.to_radians(), 16.0 / 9.0, 0.1, 500.0);
        let view = Mat4::look_to_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        Frustum::from_view_proj(proj * view)
    }

    #[test]
    fn box_in_front_is_visible() {
        let f = camera_at_origin_looking_minus_z();
        assert!(f.intersects_aabb(Vec3::new(-1.0, -1.0, -20.0), Vec3::new(1.0, 1.0, -18.0)));
    }

    #[test]
    fn box_behind_is_culled() {
        let f = camera_at_origin_looking_minus_z();
        assert!(!f.intersects_aabb(Vec3::new(-1.0, -1.0, 18.0), Vec3::new(1.0, 1.0, 20.0)));
    }

    #[test]
    fn box_beyond_far_plane_is_culled() {
        let f = camera_at_origin_looking_minus_z();
        assert!(!f.intersects_aabb(Vec3::new(-1.0, -1.0, -600.0), Vec3::new(1.0, 1.0, -590.0)));
    }

    #[test]
    fn box_far_to_the_side_is_culled() {
        let f = camera_at_origin_looking_minus_z();
        assert!(!f.intersects_aabb(Vec3::new(500.0, -1.0, -20.0), Vec3::new(502.0, 1.0, -18.0)));
    }

    #[test]
    fn box_straddling_a_plane_is_visible() {
        // Half in front of the camera, half behind: intersects ⇒ visible.
        let f = camera_at_origin_looking_minus_z();
        assert!(f.intersects_aabb(Vec3::new(-1.0, -1.0, -5.0), Vec3::new(1.0, 1.0, 5.0)));
    }

    #[test]
    fn enormous_box_containing_the_whole_frustum_is_visible() {
        let f = camera_at_origin_looking_minus_z();
        assert!(f.intersects_aabb(Vec3::splat(-10_000.0), Vec3::splat(10_000.0)));
    }
}
