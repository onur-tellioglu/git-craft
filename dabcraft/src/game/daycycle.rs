// Day/night cycle (spec §7): 20-minute full cycle; the sun angle drives the
// shader's sky/sun uniforms and the clear color. Flood-fill skylight values
// are NEVER touched — night darkening happens entirely in the shader via
// the day factor / sky color (spec §4).

use glam::Vec3;

pub struct DayCycle {
    /// Fraction of a full day in [0, 1): 0.0 sunrise, 0.25 noon,
    /// 0.5 sunset, 0.75 midnight.
    pub time: f32,
    pub cycle_secs: f32,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl DayCycle {
    pub const DEFAULT_CYCLE_SECS: f32 = 1200.0; // 20 minutes (spec §7)

    pub fn new() -> Self {
        // Start mid-morning: bright, with the sun low enough to show shading.
        Self { time: 0.1, cycle_secs: Self::DEFAULT_CYCLE_SECS }
    }

    pub fn advance(&mut self, dt: f32) {
        self.time = (self.time + dt / self.cycle_secs).rem_euclid(1.0);
    }

    /// Sun elevation in [-1, 1]: sin of the day angle.
    fn elevation(&self) -> f32 {
        (self.time * std::f32::consts::TAU).sin()
    }

    /// Sky brightness multiplier in [0.03, 1]: smoothsteps through dawn and
    /// dusk, with a small floor standing in for moonlight until M5.
    pub fn day_factor(&self) -> f32 {
        0.03 + 0.97 * smoothstep(-0.08, 0.25, self.elevation())
    }

    /// World-space direction TOWARD the sun. Rises +X (east), sets -X, on a
    /// circle tilted slightly into +Z so noon is never exactly overhead.
    pub fn sun_dir(&self) -> Vec3 {
        let a = self.time * std::f32::consts::TAU;
        Vec3::new(a.cos(), a.sin(), 0.12).normalize()
    }

    /// Linear-space sky / clear color: night → day by the day factor, with
    /// an orange band blended in near the horizon (sunrise and sunset).
    pub fn sky_color(&self) -> Vec3 {
        let day = Vec3::new(0.25, 0.55, 0.95);
        let night = Vec3::new(0.008, 0.012, 0.035);
        let base = night.lerp(day, self.day_factor());
        let glow = (1.0 - self.elevation().abs() / 0.22).clamp(0.0, 1.0);
        base.lerp(Vec3::new(0.9, 0.45, 0.2), glow * 0.45)
    }
}

impl Default for DayCycle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(time: f32) -> DayCycle {
        DayCycle { time, cycle_secs: DayCycle::DEFAULT_CYCLE_SECS }
    }

    #[test]
    fn advance_wraps_and_is_proportional() {
        let mut d = at(0.9);
        d.advance(DayCycle::DEFAULT_CYCLE_SECS * 0.2); // +0.2 of a day
        assert!((d.time - 0.1).abs() < 1e-4, "wrapped past 1.0, got {}", d.time);
    }

    #[test]
    fn noon_is_full_midnight_is_floor() {
        assert!((at(0.25).day_factor() - 1.0).abs() < 1e-3, "noon");
        assert!((at(0.75).day_factor() - 0.03).abs() < 1e-3, "midnight floor");
        assert!(at(0.5).day_factor() < 0.6, "sunset is dimmer than midday");
        assert!(at(0.05).day_factor() > at(0.0).day_factor(), "brightening after sunrise");
    }

    #[test]
    fn day_factor_stays_in_bounds_all_day() {
        for i in 0..200 {
            let f = at(i as f32 / 200.0).day_factor();
            assert!((0.03..=1.0).contains(&f), "t={i}: factor {f}");
        }
    }

    #[test]
    fn sun_rises_in_the_east_and_is_normalized() {
        let sunrise = at(0.0).sun_dir();
        assert!(sunrise.x > 0.9, "sunrise points east (+X), got {sunrise}");
        let noon = at(0.25).sun_dir();
        assert!(noon.y > 0.9, "noon is overhead");
        for i in 0..20 {
            let d = at(i as f32 / 20.0).sun_dir();
            assert!((d.length() - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn sky_color_is_blue_at_noon_dark_at_midnight_warm_at_sunset() {
        let noon = at(0.25).sky_color();
        assert!(noon.z > noon.x, "noon sky is blue-dominant");
        let midnight = at(0.75).sky_color();
        assert!(midnight.max_element() < 0.08, "midnight is near-black, got {midnight}");
        let sunset = at(0.5).sky_color();
        assert!(sunset.x > sunset.z, "sunset is warm (red over blue), got {sunset}");
    }
}
