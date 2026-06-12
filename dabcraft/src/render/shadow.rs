//! Cascade split and light-matrix math for Cascaded Shadow Maps (CSM).
//! This module is pure math — no GPU resources. It will be wired into the
//! render pipeline in a later milestone, so the public API is unused here.
#![allow(dead_code)]

use glam::{Mat4, Vec3};

pub const CASCADE_COUNT: usize = 3;
pub const SHADOW_RESOLUTION: u32 = 2048;
/// View distance covered by the cascades. Beyond it the flood-fill skylight
/// guard takes over as the only darkening term (spec §6).
pub const SHADOW_FAR: f32 = 360.0;
/// Light-space depth margin behind/before the slice sphere so casters outside
/// the camera frustum (mountains, trees up-sun) still shadow it. World height
/// is 256, so 300 covers any caster the world can contain.
const Z_MARGIN: f32 = 300.0;

/// Practical split scheme: per-boundary blend of uniform and logarithmic.
pub fn cascade_splits(near: f32, far: f32, lambda: f32) -> [f32; CASCADE_COUNT + 1] {
    let mut s = [0.0; CASCADE_COUNT + 1];
    for (i, v) in s.iter_mut().enumerate() {
        let t = i as f32 / CASCADE_COUNT as f32;
        let uniform = near + (far - near) * t;
        // Guard near == 0 (log split undefined); uniform-only there.
        let log = if near > 0.0 { near * (far / near).powf(t) } else { uniform };
        *v = uniform * (1.0 - lambda) + log * lambda;
    }
    s
}

/// World-space corners of the camera frustum slice [near_d, far_d].
/// Order: near (-x-y, +x-y, +x+y, -x+y), then far likewise.
pub fn slice_corners(
    pos: Vec3,
    forward: Vec3,
    fov_y: f32,
    aspect: f32,
    near_d: f32,
    far_d: f32,
) -> [Vec3; 8] {
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let tan_half = (fov_y * 0.5).tan();
    let mut out = [Vec3::ZERO; 8];
    for (half, &d) in [near_d, far_d].iter().enumerate() {
        let hh = tan_half * d;
        let hw = hh * aspect;
        let c = pos + forward * d;
        out[half * 4] = c - right * hw - up * hh;
        out[half * 4 + 1] = c + right * hw - up * hh;
        out[half * 4 + 2] = c + right * hw + up * hh;
        out[half * 4 + 3] = c - right * hw + up * hh;
    }
    out
}

pub struct CascadeFit {
    pub view_proj: Mat4,
    /// World size of one shadow-map texel (normal-offset bias scale).
    pub texel_world: f32,
}

/// Orthographic light matrix around the slice's bounding sphere, with the XY
/// translation snapped to whole shadow-map texels.
pub fn fit_light_matrix(corners: &[Vec3; 8], light_dir: Vec3, resolution: u32) -> CascadeFit {
    let center = corners.iter().copied().sum::<Vec3>() / 8.0;
    let radius = corners
        .iter()
        .map(|c| (*c - center).length())
        .fold(0.0f32, f32::max)
        .max(1.0);
    // Light "up" is +Z: sun/moon directions always carry a fixed ±0.119 Z
    // tilt (DayCycle::sun_dir), so they are never parallel to Z.
    let eye = center + light_dir * (radius + Z_MARGIN);
    let view = Mat4::look_to_rh(eye, -light_dir, Vec3::Z);
    let depth = 2.0 * (radius + Z_MARGIN);
    let proj = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, depth);
    let vp = proj * view;
    // Snap: move the projection so the world origin lands on a texel corner;
    // every world point then lands on the same sub-texel phase each frame.
    let half_res = resolution as f32 / 2.0;
    let origin = vp.project_point3(Vec3::ZERO);
    let snap = Mat4::from_translation(Vec3::new(
        ((origin.x * half_res).round() - origin.x * half_res) / half_res,
        ((origin.y * half_res).round() - origin.y * half_res) / half_res,
        0.0,
    ));
    CascadeFit { view_proj: snap * vp, texel_world: 2.0 * radius / resolution as f32 }
}

/// Update cadence (spec §6: far cascades every 2–4 frames).
pub fn cascade_due(frame: u64, cascade: usize) -> bool {
    match cascade {
        0 => true,
        1 => frame.is_multiple_of(2),
        _ => frame.is_multiple_of(4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: u32 = SHADOW_RESOLUTION;

    fn light() -> Vec3 {
        Vec3::new(1.0, 1.0, 0.12).normalize()
    }

    #[test]
    fn splits_cover_the_range_monotonically() {
        let s = cascade_splits(0.5, SHADOW_FAR, 0.7);
        assert_eq!(s[0], 0.5);
        assert_eq!(s[CASCADE_COUNT], SHADOW_FAR);
        for i in 0..CASCADE_COUNT {
            assert!(s[i] < s[i + 1], "splits not increasing: {s:?}");
        }
    }

    #[test]
    fn lambda_zero_gives_uniform_splits() {
        let s = cascade_splits(0.0, 300.0, 0.0);
        assert!((s[1] - 100.0).abs() < 1e-3 && (s[2] - 200.0).abs() < 1e-3, "{s:?}");
    }

    #[test]
    fn slice_corners_match_hand_computed_frustum() {
        // fov 90° (tan = 1), aspect 1, looking down -Z from the origin:
        // near plane at 10 has corners (±10, ±10, -10).
        let c = slice_corners(Vec3::ZERO, Vec3::NEG_Z, 90f32.to_radians(), 1.0, 10.0, 20.0);
        let expect_near = [
            Vec3::new(-10.0, -10.0, -10.0),
            Vec3::new(10.0, -10.0, -10.0),
            Vec3::new(10.0, 10.0, -10.0),
            Vec3::new(-10.0, 10.0, -10.0),
        ];
        for (got, want) in c[..4].iter().zip(expect_near) {
            assert!((*got - want).length() < 1e-3, "{got} != {want}");
        }
        assert!((c[4] - Vec3::new(-20.0, -20.0, -20.0)).length() < 1e-3, "far corner: {}", c[4]);
    }

    #[test]
    fn light_matrix_contains_every_slice_corner() {
        let corners = slice_corners(
            Vec3::new(100.0, 80.0, -40.0),
            Vec3::new(0.6, -0.3, 0.74).normalize(),
            70f32.to_radians(),
            1.6,
            32.0,
            128.0,
        );
        let fit = fit_light_matrix(&corners, light(), RES);
        for c in corners {
            let ndc = fit.view_proj.project_point3(c);
            assert!(ndc.x.abs() <= 1.001 && ndc.y.abs() <= 1.001, "corner outside XY: {ndc}");
            assert!((0.0..=1.0).contains(&ndc.z), "corner outside depth: {ndc}");
        }
    }

    #[test]
    fn texel_snap_quantizes_the_world_origin() {
        // Two slightly different camera positions must produce light matrices
        // whose texel grids coincide: the world origin always projects onto
        // an integer texel coordinate.
        for dx in [0.0, 0.013, 1.77] {
            let corners = slice_corners(
                Vec3::new(50.0 + dx, 70.0, 10.0),
                Vec3::NEG_Z,
                70f32.to_radians(),
                1.6,
                0.5,
                32.0,
            );
            let fit = fit_light_matrix(&corners, light(), RES);
            let t = fit.view_proj.project_point3(Vec3::ZERO);
            let tx = t.x * RES as f32 / 2.0;
            let ty = t.y * RES as f32 / 2.0;
            assert!((tx - tx.round()).abs() < 1e-3, "X off-grid by {} texels", tx - tx.round());
            assert!((ty - ty.round()).abs() < 1e-3, "Y off-grid by {} texels", ty - ty.round());
        }
    }

    #[test]
    fn texel_world_size_matches_the_ortho_diameter() {
        let corners = slice_corners(Vec3::ZERO, Vec3::NEG_Z, 70f32.to_radians(), 1.6, 0.5, 32.0);
        let center = corners.iter().copied().sum::<Vec3>() / 8.0;
        let radius = corners.iter().map(|c| (*c - center).length()).fold(0.0f32, f32::max);
        let fit = fit_light_matrix(&corners, light(), RES);
        assert!((fit.texel_world - 2.0 * radius / RES as f32).abs() < 1e-5);
    }

    #[test]
    fn cascade_cadence_matches_the_spec() {
        // Spec §6: near cascade every frame, far cascades every 2–4 frames.
        for f in 1..=8u64 {
            assert!(cascade_due(f, 0));
        }
        assert!(cascade_due(2, 1) && !cascade_due(3, 1));
        assert!(cascade_due(4, 2) && !cascade_due(5, 2) && !cascade_due(6, 2) && !cascade_due(7, 2));
    }
}
