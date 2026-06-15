//! Deterministic `--bench` harness.
//!
//! `cargo run --release -- --bench` runs a fixed-route flythrough at the full
//! render distance, warms up until streaming is idle, records a window of
//! frames, and prints frame-time / GPU-time percentiles against the 120 fps
//! budget. All the pure logic — percentiles, the orbit route, the
//! warmup→record→done state machine, the report text, and CLI parsing — lives
//! here and is unit-tested. `app.rs` and `gpu.rs` hold only the wgpu/winit glue.

use std::f32::consts::TAU;

/// Recorded-frame count when `--bench-frames` is omitted.
pub const DEFAULT_FRAMES: usize = 600;
/// Consecutive idle (streaming-quiet) frames required before recording starts.
const STEADY_FRAMES: u32 = 30;
/// Hard cap on warmup frames so a world that never goes idle can't hang the run.
/// When this fires, recording starts even if streaming is still active — the
/// baseline may be slightly pessimistic (a few streaming jobs still in flight),
/// but it prevents an infinite hang. This is intentional: a fully loaded world
/// would idle naturally; WARMUP_CAP is the escape hatch for pathological cases.
const WARMUP_CAP: u32 = 6000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BenchConfig {
    /// Number of frames to record after warmup.
    pub frames: usize,
    /// When true, the bench window is sized to the primary display's native
    /// physical pixel resolution instead of the hardcoded 1280×720.
    /// Activated by `--native-bench` (requires `--bench`).
    pub native_res: bool,
}

/// Parse the process args. Returns `Some` iff `--bench` is present. Reads an
/// optional `--bench-frames N` (a missing or unparseable/zero N keeps the
/// default). The program name in `args[0]` is ignored naturally.
pub fn parse_bench_args(args: impl Iterator<Item = String>) -> Option<BenchConfig> {
    let args: Vec<String> = args.collect();
    if !args.iter().any(|a| a == "--bench") {
        return None;
    }
    let mut frames = DEFAULT_FRAMES;
    let mut native_res = false;
    for i in 0..args.len() {
        if args[i] == "--bench-frames"
            && let Some(n) = args.get(i + 1).and_then(|v| v.parse::<usize>().ok())
            && n > 0
        {
            frames = n;
        }
        if args[i] == "--native-bench" {
            native_res = true;
        }
    }
    Some(BenchConfig { frames, native_res })
}

/// Linearly-interpolated percentile (`p` in 0..=1) of an ascending-sorted slice.
/// Empty → 0.0; single element → that element.
pub fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f32;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f32;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

/// Aggregate statistics over a sample series.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub frames: usize,
    pub min: f32,
    pub mean: f32,
    pub p50: f32,
    pub p95: f32,
    pub p99: f32,
    pub max: f32,
}

/// Summarize a sample series. `None` for an empty series.
pub fn summarize(samples: &[f32]) -> Option<Summary> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sum: f32 = sorted.iter().sum();
    Some(Summary {
        frames: sorted.len(),
        min: sorted[0],
        max: sorted[sorted.len() - 1],
        mean: sum / sorted.len() as f32,
        p50: percentile(&sorted, 0.50),
        p95: percentile(&sorted, 0.95),
        p99: percentile(&sorted, 0.99),
    })
}

/// Yaw (radians) for the orbit route at recorded frame `frame` of `total`.
/// Sweeps a full turn across the recorded window so frustum culling is
/// exercised in every direction.
///
/// Edge cases:
/// - `total == 0` → returns 0.0 (no division by zero).
/// - `frame > total` → clamped to `total`, yielding TAU. In practice the
///   recording loop only calls this with `frame` in `0..total-1`, so TAU is
///   clamped-but-unreachable during a normal run.
pub fn bench_yaw(frame: usize, total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }
    (frame.min(total) as f32) / total as f32 * TAU
}

/// Build the printed report and PASS/FAIL verdict.
///
/// The verdict always uses **CPU frame-time p99**. In `Immediate` present mode
/// the CPU `dt` is frame-coherent and reflects uncapped render cost directly.
///
/// GPU percentiles are printed as a **diagnostic only**. On Apple TBDR the
/// async readback lags 2-3 frames (`max_frame_latency = 2`), so per-frame GPU
/// values are not frame-coherent with the CPU samples. Additionally, summing
/// per-pass timestamps overcounts true frame time on TBDR because render-pass
/// stages pipeline/overlap (measured example: 27.5 ms summed GPU vs 9.86 ms
/// actual at 101 fps). Use the GPU column to identify hotspot passes, not to
/// evaluate the overall frame budget. A frame-coherent GPU total is tracked in
/// issue #18.
pub fn format_report(
    cpu: &Summary,
    gpu: &Summary,
    target_fps: f32,
    timestamps: bool,
    render_radius: i32,
    resolution: &str,
) -> String {
    let budget_ms = 1000.0 / target_fps;
    // Verdict is always CPU p99 — it is frame-coherent under Immediate present.
    let verdict = if cpu.p99 <= budget_ms { "PASS" } else { "FAIL" };
    let mut s = String::new();
    // Distinguish native-resolution runs in the header so two bench outputs
    // saved to disk are unambiguous at a glance without opening the body.
    // The resolution line already carries the exact pixel size; this label is
    // a human-readable shorthand. 1280×720 is the fixed baseline resolution;
    // anything else means --native-bench was used.
    let header = if resolution == "1280×720" {
        "=== git-craft --bench ==="
    } else {
        "=== git-craft bench (native-res) ==="
    };
    s.push_str(&format!("{header}\n"));
    s.push_str(&format!("resolution:       {resolution}\n"));
    s.push_str(&format!(
        "render distance:  {render_radius} columns ({}-chunk diameter, {} blocks)\n",
        render_radius * 2,
        render_radius * 32,
    ));
    s.push_str(&format!("frames recorded:  {}\n", cpu.frames));
    s.push_str(&format!(
        "target:           {target_fps:.0} fps ({budget_ms:.2} ms/frame)\n"
    ));
    s.push_str("                    min   mean    p50    p95    p99    max\n");
    s.push_str(&format!(
        "CPU frame ms     {:6.2} {:6.2} {:6.2} {:6.2} {:6.2} {:6.2}\n",
        cpu.min, cpu.mean, cpu.p50, cpu.p95, cpu.p99, cpu.max,
    ));
    if timestamps {
        // GPU column is a diagnostic reference — NOT the verdict driver.
        // Async readback lags 2+ frames and per-pass values overlap on Apple
        // TBDR; these numbers are useful for identifying hotspot passes but
        // must not be compared directly to the frame budget. See issue #18.
        s.push_str(&format!(
            "GPU frame ms     {:6.2} {:6.2} {:6.2} {:6.2} {:6.2} {:6.2}  [diagnostic, not verdict — see #18]\n",
            gpu.min, gpu.mean, gpu.p50, gpu.p95, gpu.p99, gpu.max,
        ));
    } else {
        s.push_str("GPU frame ms     (TIMESTAMP_QUERY unavailable)\n");
    }
    s.push_str(&format!(
        "verdict: {verdict} (CPU p99 {:.2} ms vs {budget_ms:.2} ms budget)\n",
        cpu.p99
    ));
    s
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Warmup,
    Recording,
    Done,
}

/// Warmup → record → done state machine over plain frame counters. The app
/// drives it: `warmup_step` each frame until it returns true, then `push` one
/// `(cpu_ms, gpu_ms)` sample per frame until `is_done`.
pub struct BenchRun {
    config: BenchConfig,
    phase: Phase,
    warmup_frames: u32,
    idle_streak: u32,
    cpu_ms: Vec<f32>,
    gpu_ms: Vec<f32>,
}

impl BenchRun {
    pub fn new(config: BenchConfig) -> Self {
        Self {
            config,
            phase: Phase::Warmup,
            warmup_frames: 0,
            idle_streak: 0,
            cpu_ms: Vec::with_capacity(config.frames),
            gpu_ms: Vec::with_capacity(config.frames),
        }
    }

    pub fn frames(&self) -> usize {
        self.config.frames
    }
    pub fn is_warming(&self) -> bool {
        self.phase == Phase::Warmup
    }
    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }
    pub fn recorded(&self) -> usize {
        self.cpu_ms.len()
    }

    /// Advance warmup by one frame. `idle` = streaming quiet this frame.
    /// Returns true once recording should begin: `STEADY_FRAMES` consecutive
    /// idle frames, or the warmup cap. Transitions the phase on the way out.
    pub fn warmup_step(&mut self, idle: bool) -> bool {
        debug_assert_eq!(self.phase, Phase::Warmup);
        self.warmup_frames += 1;
        if idle {
            self.idle_streak += 1;
        } else {
            self.idle_streak = 0;
        }
        let ready = self.idle_streak >= STEADY_FRAMES || self.warmup_frames >= WARMUP_CAP;
        if ready {
            self.phase = if self.config.frames == 0 {
                Phase::Done
            } else {
                Phase::Recording
            };
        }
        ready
    }

    /// Record one frame's CPU + GPU milliseconds. Transitions to Done at the
    /// configured frame count. No-op unless currently recording.
    ///
    /// GPU samples where `gpu_ms == 0.0` are skipped. A zero reading means
    /// the async readback has not completed yet (the timestamp buffer is
    /// initialized to 0 and only updated after a successful `after_submit`
    /// readback, which lags 2+ frames). Stale zeros would pull down the GPU
    /// diagnostic distribution without conveying any useful information.
    /// CPU samples are always recorded — they are never stale.
    pub fn push(&mut self, cpu_ms: f32, gpu_ms: f32) {
        if self.phase != Phase::Recording {
            return;
        }
        self.cpu_ms.push(cpu_ms);
        if gpu_ms > 0.0 {
            self.gpu_ms.push(gpu_ms);
        }
        if self.cpu_ms.len() >= self.config.frames {
            self.phase = Phase::Done;
        }
    }

    pub fn cpu_summary(&self) -> Option<Summary> {
        summarize(&self.cpu_ms)
    }
    pub fn gpu_summary(&self) -> Option<Summary> {
        summarize(&self.gpu_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "expected {b}, got {a}");
    }

    #[test]
    fn percentile_interpolates_between_ranks() {
        let s: Vec<f32> = (1..=10).map(|n| n as f32).collect();
        approx(percentile(&s, 0.50), 5.5);
        approx(percentile(&s, 0.95), 9.55);
        approx(percentile(&s, 0.99), 9.91);
        approx(percentile(&s, 0.0), 1.0);
        approx(percentile(&s, 1.0), 10.0);
    }

    #[test]
    fn percentile_handles_degenerate_inputs() {
        approx(percentile(&[], 0.5), 0.0);
        approx(percentile(&[42.0], 0.99), 42.0);
        // p is clamped, never panics out of range.
        approx(percentile(&[1.0, 2.0], 2.0), 2.0);
    }

    #[test]
    fn summarize_computes_known_stats() {
        let s: Vec<f32> = (1..=10).map(|n| n as f32).collect();
        let sum = summarize(&s).unwrap();
        assert_eq!(sum.frames, 10);
        approx(sum.min, 1.0);
        approx(sum.max, 10.0);
        approx(sum.mean, 5.5);
        approx(sum.p50, 5.5);
        approx(sum.p99, 9.91);
    }

    #[test]
    fn summarize_empty_is_none_single_is_flat() {
        assert!(summarize(&[]).is_none());
        let one = summarize(&[7.0]).unwrap();
        approx(one.min, 7.0);
        approx(one.max, 7.0);
        approx(one.p99, 7.0);
    }

    #[test]
    fn bench_yaw_sweeps_a_full_turn() {
        approx(bench_yaw(0, 100), 0.0);
        approx(bench_yaw(50, 100), std::f32::consts::PI);
        // The actually-exercised near-end boundary: last recorded frame is total-1.
        approx(bench_yaw(99, 100), TAU * 99.0 / 100.0);
        // bench_yaw(total, total) == TAU via the clamp, but the recording loop
        // never reaches this frame — it's a guard, not a reachable case.
        approx(bench_yaw(100, 100), TAU);
        // Out-of-range beyond total: also clamped to TAU.
        approx(bench_yaw(200, 100), TAU);
        // Zero total: guard against division by zero.
        approx(bench_yaw(5, 0), 0.0);
    }

    #[test]
    fn warmup_needs_a_consecutive_idle_streak() {
        let mut run = BenchRun::new(BenchConfig {
            frames: 5,
            native_res: false,
        });
        for _ in 0..(STEADY_FRAMES - 1) {
            assert!(!run.warmup_step(true));
        }
        assert!(run.warmup_step(true)); // STEADY_FRAMES-th idle
        assert!(!run.is_warming()); // transitioned to recording
    }

    #[test]
    fn a_busy_frame_resets_the_idle_streak() {
        let mut run = BenchRun::new(BenchConfig {
            frames: 5,
            native_res: false,
        });
        for _ in 0..(STEADY_FRAMES - 1) {
            run.warmup_step(true);
        }
        assert!(!run.warmup_step(false)); // reset just before the threshold
        for _ in 0..(STEADY_FRAMES - 1) {
            assert!(!run.warmup_step(true));
        }
        assert!(run.warmup_step(true));
    }

    #[test]
    fn warmup_cap_forces_recording_when_never_idle() {
        let mut run = BenchRun::new(BenchConfig {
            frames: 5,
            native_res: false,
        });
        let mut ready = false;
        for _ in 0..WARMUP_CAP {
            ready = run.warmup_step(false);
            if ready {
                break;
            }
        }
        assert!(ready);
        assert!(!run.is_warming());
    }

    /// A world that has brief idle windows but never sustains STEADY_FRAMES
    /// consecutive idles must still exit warmup when WARMUP_CAP is reached.
    /// This guards the streak-reset + cap interaction: the idle_streak resets
    /// repeatedly but warmup_frames keeps climbing, so the cap eventually fires.
    #[test]
    fn warmup_cap_fires_despite_recurring_busy_frames() {
        let mut run = BenchRun::new(BenchConfig {
            frames: 5,
            native_res: false,
        });
        let mut ready = false;
        for frame in 0..WARMUP_CAP as usize {
            // Pattern: STEADY_FRAMES-1 idle frames, then 1 busy frame.
            // The idle streak never reaches STEADY_FRAMES; the busy frame resets
            // it each cycle. Only WARMUP_CAP can break the stalemate.
            let idle = (frame % STEADY_FRAMES as usize) != (STEADY_FRAMES as usize - 1);
            ready = run.warmup_step(idle);
            if ready {
                break;
            }
        }
        assert!(ready);
        assert!(!run.is_warming());
    }

    #[test]
    fn push_records_until_done() {
        let mut run = BenchRun::new(BenchConfig {
            frames: 3,
            native_res: false,
        });
        while !run.warmup_step(true) {}
        run.push(8.0, 4.0);
        run.push(9.0, 5.0);
        assert!(!run.is_done());
        run.push(7.0, 3.0);
        assert!(run.is_done());
        assert_eq!(run.recorded(), 3);
        // Extra pushes after Done are ignored.
        run.push(99.0, 99.0);
        assert_eq!(run.recorded(), 3);
        approx(run.gpu_summary().unwrap().min, 3.0);
    }

    /// Stale zero GPU samples (readback not yet completed) must not pollute the
    /// GPU diagnostic distribution. The first two frames have gpu_ms == 0 (the
    /// async readback hasn't landed yet); only the third has a real value.
    /// cpu_ms is always recorded regardless of the GPU value.
    #[test]
    fn push_skips_stale_zero_gpu_samples() {
        let mut run = BenchRun::new(BenchConfig {
            frames: 3,
            native_res: false,
        });
        while !run.warmup_step(true) {}
        run.push(8.0, 0.0); // stale — gpu not yet readback
        run.push(9.0, 0.0); // stale
        run.push(7.0, 5.0); // real readback
        assert!(run.is_done());
        assert_eq!(run.recorded(), 3); // cpu always recorded
        // GPU summary should contain only the one non-zero sample.
        let gpu = run.gpu_summary().unwrap();
        assert_eq!(gpu.frames, 1, "only 1 non-zero GPU sample expected");
        approx(gpu.min, 5.0);
        approx(gpu.max, 5.0);
    }

    #[test]
    fn report_verdict_always_uses_cpu_p99() {
        let fast = Summary {
            frames: 600,
            min: 2.0,
            mean: 3.0,
            p50: 3.0,
            p95: 4.0,
            p99: 5.0,
            max: 6.0,
        };
        let slow = Summary { p99: 20.0, ..fast };
        // Verdict is always CPU p99, regardless of whether GPU timestamps are
        // available. GPU async readback lags 2+ frames on Apple TBDR and
        // per-pass values overlap, making the GPU column an unreliable verdict
        // driver (it produced false FAILs in v0.6.1 — see PR #17).

        // CPU fast (5 ms < 8.33 ms), GPU slow (20 ms) + timestamps → PASS
        // because the verdict ignores GPU.
        let pass_ts = format_report(&fast, &slow, 120.0, true, 12, "1280×720");
        assert!(pass_ts.contains("PASS"), "{pass_ts}");
        assert!(pass_ts.contains("CPU p99"), "{pass_ts}");

        // CPU slow (20 ms > 8.33 ms), GPU fast (5 ms) + timestamps → FAIL
        // because the verdict ignores GPU.
        let fail_ts = format_report(&slow, &fast, 120.0, true, 12, "1280×720");
        assert!(fail_ts.contains("FAIL"), "{fail_ts}");
        assert!(fail_ts.contains("CPU p99"), "{fail_ts}");

        // GPU diagnostic line is present when timestamps are available.
        assert!(pass_ts.contains("diagnostic"), "{pass_ts}");

        // No timestamps → CPU p99 verdict, GPU line shows unavailable.
        let no_ts = format_report(&slow, &fast, 120.0, false, 12, "1280×720");
        assert!(no_ts.contains("FAIL"), "{no_ts}");
        assert!(no_ts.contains("TIMESTAMP_QUERY unavailable"), "{no_ts}");
        assert!(no_ts.contains("CPU p99"), "{no_ts}");
    }

    #[test]
    fn parse_bench_args_detects_native_bench_flag() {
        // --native-bench implies --bench and sets native_res.
        let cfg = parse_bench_args(
            ["prog", "--bench", "--native-bench"]
                .map(String::from)
                .into_iter(),
        )
        .unwrap();
        assert!(cfg.native_res, "native_res should be true");
        assert_eq!(cfg.frames, DEFAULT_FRAMES);
    }

    #[test]
    fn native_bench_without_bench_is_none() {
        // --native-bench alone does NOT activate bench mode (--bench required).
        let result = parse_bench_args(["prog", "--native-bench"].map(String::from).into_iter());
        assert!(result.is_none());
    }

    #[test]
    fn native_bench_combines_with_bench_frames() {
        let cfg = parse_bench_args(
            ["prog", "--bench", "--native-bench", "--bench-frames", "100"]
                .map(String::from)
                .into_iter(),
        )
        .unwrap();
        assert!(cfg.native_res);
        assert_eq!(cfg.frames, 100);
    }

    #[test]
    fn non_native_bench_has_native_res_false() {
        let cfg = parse_bench_args(["prog", "--bench"].map(String::from).into_iter()).unwrap();
        assert!(!cfg.native_res);
    }

    #[test]
    fn format_report_includes_resolution_label() {
        let s = Summary {
            frames: 10,
            min: 1.0,
            mean: 2.0,
            p50: 2.0,
            p95: 3.0,
            p99: 4.0,
            max: 5.0,
        };
        let r = format_report(&s, &s, 120.0, false, 12, "1280×720");
        assert!(
            r.contains("1280×720"),
            "report should contain resolution: {r}"
        );
    }

    #[test]
    fn parse_bench_args_detects_flag_and_frames() {
        let none = parse_bench_args(["prog".to_string()].into_iter());
        assert!(none.is_none());
        let default = parse_bench_args(["prog".into(), "--bench".into()].into_iter()).unwrap();
        assert_eq!(default.frames, DEFAULT_FRAMES);
        let custom = parse_bench_args(
            ["prog", "--bench", "--bench-frames", "120"]
                .map(String::from)
                .into_iter(),
        )
        .unwrap();
        assert_eq!(custom.frames, 120);
        // Bad frame count falls back to the default.
        let bad = parse_bench_args(
            ["prog", "--bench", "--bench-frames", "oops"]
                .map(String::from)
                .into_iter(),
        )
        .unwrap();
        assert_eq!(bad.frames, DEFAULT_FRAMES);
        // No --bench → None even with --bench-frames present.
        assert!(
            parse_bench_args(
                ["prog", "--bench-frames", "9"]
                    .map(String::from)
                    .into_iter()
            )
            .is_none()
        );
    }
}
