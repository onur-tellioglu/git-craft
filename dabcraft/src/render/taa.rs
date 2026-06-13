//! Temporal anti-aliasing: sub-pixel jitter sequence and the resolve pass.

/// Halton(base) value for index i (i >= 1). Radical-inverse digit expansion.
fn halton(mut i: u32, base: u32) -> f32 {
    let mut f = 1.0;
    let mut r = 0.0;
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// Sub-pixel jitter offset in pixels, in [-0.5, 0.5], for frame `n`.
/// Halton(2,3) recentred to zero so the sequence has no DC bias.
pub fn jitter_offset(frame: u64) -> (f32, f32) {
    let i = (frame % JITTER_PERIOD) as u32 + 1;
    (halton(i, 2) - 0.5, halton(i, 3) - 0.5)
}

/// Length of the jitter cycle. 8 gives a good 1-frame-per-subsample spread
/// without a long convergence tail.
pub const JITTER_PERIOD: u64 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halton_matches_known_values() {
        assert!((halton(1, 2) - 0.5).abs() < 1e-6);
        assert!((halton(2, 2) - 0.25).abs() < 1e-6);
        assert!((halton(3, 2) - 0.75).abs() < 1e-6);
        assert!((halton(1, 3) - 1.0 / 3.0).abs() < 1e-6);
        assert!((halton(2, 3) - 2.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn offsets_stay_in_half_pixel_and_recentre() {
        let mut sx = 0.0;
        let mut sy = 0.0;
        for f in 0..JITTER_PERIOD {
            let (x, y) = jitter_offset(f);
            assert!((-0.5..=0.5).contains(&x) && (-0.5..=0.5).contains(&y), "f={f}: {x},{y}");
            sx += x;
            sy += y;
        }
        // Recentred sequence has near-zero mean over a period (no DC drift).
        assert!(sx.abs() / (JITTER_PERIOD as f32) < 0.2, "mean x {sx}");
        assert!(sy.abs() / (JITTER_PERIOD as f32) < 0.2, "mean y {sy}");
    }
}
