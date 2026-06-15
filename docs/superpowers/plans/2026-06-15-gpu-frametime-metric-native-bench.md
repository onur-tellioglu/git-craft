---
title: True GPU Frame-Time Metric + Native-Resolution Bench Mode
date: 2026-06-15
domain: tooling
type: fix
priority: high
breaking: false
db-migration: false
rls-affecting: false
slice: null
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files:
  - git-craft/src/render/timestamps.rs
  - git-craft/src/bench.rs
  - git-craft/src/app.rs
  - CHANGELOG.md
trigger-tasks-touched: []
shared-modules-touched:
  - git-craft/src/render/timestamps.rs
optimization-required: false
security-required: false
---

# True GPU Frame-Time Metric + Native-Resolution Bench Mode

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the overcounting per-pass-sum GPU metric with the true whole-frame GPU wall-clock (`last_pass_end − first_pass_begin`), and add a `--native-bench` flag that runs the existing deterministic flythrough at the primary display's native pixel resolution.

**Architecture:** The timestamp buffer already holds `2 × N` raw u64 ticks (begin/end per pass). A new pure function `frame_wall_ms(ticks, period_ns)` picks the global min begin and max end, giving true elapsed GPU wall-clock regardless of TBDR pass pipelining. `GpuTimer::total_ms()` is replaced by `GpuTimer::frame_wall_ms()` which calls this function. The `--native-bench` flag extends `BenchConfig` with a boolean; `app.rs` reads the primary monitor size at window creation time when that flag is set.

**Tech Stack:** Rust (edition 2024), wgpu 29, winit, `bytemuck`.

---

## Context: why the sum is wrong on Apple TBDR

On tile-based deferred renderers (all Apple Silicon GPUs), the GPU pipelines consecutive render passes: pass N's fragment phase runs in parallel with pass N+1's vertex/tile phase. The GPU timestamp hardware records the real start/end of each pass's own work; summing the deltas therefore double-counts the overlapping intervals and inflates the reported total by ~3×.

**Measured evidence (v0.6.1, 1280×720):**
- Bench `--bench` GPU p99 (sum): **13.48 ms** → implied ~74 fps
- Actual observed FPS during the same bench run: **~101 fps** (≈ 9.9 ms/frame)
- HUD showed `GPU ms: 27.50` while F3 FPS read **101** — the summed metric is ~2.8× the real frame time.

**Fix:** whole-frame wall-clock = `max(all_end_ticks) − min(all_begin_ticks)`. This is the single number that matches what the user actually sees.

---

## File map

| File | Change |
|---|---|
| `git-craft/src/render/timestamps.rs` | Add `frame_wall_ms(ticks, period_ns) → Option<f32>` pure function; add `GpuTimer::frame_wall_ms() → f32` method that calls it; deprecate (remove) `GpuTimer::total_ms()`. |
| `git-craft/src/bench.rs` | Extend `BenchConfig` with `native_res: bool`; extend `parse_bench_args` to detect `--native-bench`; update `format_report` to note the metric is wall-clock. |
| `git-craft/src/app.rs` | Replace every `total_ms()` call with `frame_wall_ms()`; add native-res window sizing branch in `resumed()`; keep per-pass `pass_ms` for HUD breakdown unchanged. |
| `CHANGELOG.md` | Add `[Unreleased]` section. |

---

## Task 1: `frame_wall_ms` pure function (TDD)

**Files:**
- Modify: `git-craft/src/render/timestamps.rs` (tests section near line 189, and add the function after `pass_millis`)

### Background

`pass_millis` (line 7–12) converts the raw `ticks` slice into per-pass deltas. The same slice is available when the readback completes (`after_submit`, line 170–176). A new function over that raw tick slice gives the wall-clock total.

The raw tick layout: `ticks[2*i]` = begin of pass `i`, `ticks[2*i+1]` = end of pass `i`. On Metal, a skipped pass may leave stale zeros; any tick that is zero must be excluded from the min/max to avoid anchoring the wall-clock to time 0. Return `None` if no valid pair survives (mirrors `pass_millis`'s `None` sentinel).

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` block in `git-craft/src/render/timestamps.rs` (after the existing `timestamp_period_scales_the_result` test, before the closing `}`):

```rust
    #[test]
    fn frame_wall_ms_uses_first_begin_and_last_end() {
        // Two passes at 1 ns/tick period:
        //   pass 0: begin=100, end=1_100_100  (1.0001 ms)
        //   pass 1: begin=500_000, end=2_600_100  (2.1001 ms)
        // Sum of deltas = 3.1002 ms; wall-clock = last_end − first_begin
        //   = 2_600_100 − 100 = 2_600_000 ticks = 2.6 ms
        let ticks = [100u64, 1_100_100, 500_000, 2_600_100];
        let result = frame_wall_ms(&ticks, 1.0);
        assert!((result.unwrap() - 2.6).abs() < 1e-3, "got {:?}", result);
    }

    #[test]
    fn frame_wall_ms_skips_zero_ticks() {
        // Pass 0 was skipped (zeros); only pass 1 ran.
        // Wall-clock must not anchor to tick 0.
        let ticks = [0u64, 0, 1_000_000, 3_000_000];
        let result = frame_wall_ms(&ticks, 1.0);
        // 3_000_000 − 1_000_000 = 2_000_000 ticks × 1 ns = 2.0 ms
        assert!((result.unwrap() - 2.0).abs() < 1e-3, "got {:?}", result);
    }

    #[test]
    fn frame_wall_ms_returns_none_when_all_ticks_are_zero() {
        // All passes skipped — no valid data.
        let ticks = [0u64, 0, 0, 0];
        assert!(frame_wall_ms(&ticks, 1.0).is_none());
    }

    #[test]
    fn frame_wall_ms_returns_none_for_empty_slice() {
        assert!(frame_wall_ms(&[], 1.0).is_none());
    }

    #[test]
    fn frame_wall_ms_period_scales_result() {
        // One pass: begin=0, end=1000 ticks; period=41.7 ns → 41.7 µs = 0.0417 ms
        let ticks = [0u64, 1000];
        // begin=0 is a valid begin (pass ran from tick 0)? No — tick 0 is ambiguous.
        // Use begin=1 to disambiguate from "skipped":
        let ticks2 = [1u64, 1001];
        let result = frame_wall_ms(&ticks2, 41.7);
        assert!((result.unwrap() - 0.0417).abs() < 1e-6, "got {:?}", result);
    }

    #[test]
    fn frame_wall_ms_single_valid_pass_equals_pass_millis() {
        // For a single non-zero pass, wall-clock == per-pass delta.
        let ticks = [500u64, 1_500_500];
        let wall = frame_wall_ms(&ticks, 1.0).unwrap();
        let per_pass = pass_millis(&ticks, 1.0)[0].unwrap();
        assert!((wall - per_pass).abs() < 1e-6);
    }

    #[test]
    fn old_sum_overcounts_overlapping_passes() {
        // Demonstrate that summing per-pass deltas gives a LARGER number than
        // frame_wall_ms when passes overlap (which they do on TBDR).
        // pass 0: [100, 1_100_100]; pass 1: [500_000, 2_600_100]
        // pass 1 begins before pass 0 ends → overlap → sum > wall
        let ticks = [100u64, 1_100_100, 500_000, 2_600_100];
        let sum_ms: f32 = pass_millis(&ticks, 1.0)
            .into_iter()
            .flatten()
            .sum();
        let wall_ms = frame_wall_ms(&ticks, 1.0).unwrap();
        assert!(sum_ms > wall_ms, "sum={sum_ms} should exceed wall={wall_ms}");
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

Run from `git-craft/`:
```bash
cargo test -p git-craft render::timestamps 2>&1 | tail -20
```
Expected: compile error — `frame_wall_ms` not found.

- [ ] **Step 3: Implement `frame_wall_ms`**

In `git-craft/src/render/timestamps.rs`, add this function immediately after the `pass_millis` function (after line 12, before the `GpuTimer` struct doc comment):

```rust
/// Compute the true whole-frame GPU wall-clock from the raw timestamp buffer.
/// Returns `(max_end_tick − min_begin_tick) × period_ns / 1_000_000` ms.
///
/// On Apple TBDR, consecutive passes pipeline (pass N fragment overlaps pass
/// N+1 tiler), so summing per-pass deltas overcounts. The wall-clock is the
/// only accurate total.
///
/// Zero ticks are treated as "pass did not run" and are excluded from the
/// min/max reduction. Returns `None` if no valid (non-zero begin, non-zero end,
/// end > begin) pair exists.
pub fn frame_wall_ms(ticks: &[u64], period_ns: f32) -> Option<f32> {
    let mut min_begin = u64::MAX;
    let mut max_end = 0u64;
    for pair in ticks.chunks_exact(2) {
        let (begin, end) = (pair[0], pair[1]);
        // Skip pairs where either tick is zero (pass was not issued this frame)
        // or where end ≤ begin (Metal occasional bad sample).
        if begin == 0 || end == 0 || end <= begin {
            continue;
        }
        if begin < min_begin {
            min_begin = begin;
        }
        if end > max_end {
            max_end = end;
        }
    }
    if max_end == 0 || min_begin == u64::MAX || max_end <= min_begin {
        return None;
    }
    Some((max_end - min_begin) as f32 * period_ns / 1_000_000.0)
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test -p git-craft render::timestamps 2>&1 | tail -20
```
Expected: all tests in `render::timestamps` pass, including the 6 new ones.

- [ ] **Step 5: Add `GpuTimer::frame_wall_ms()` method and remove `total_ms()`**

In `git-craft/src/render/timestamps.rs`, replace the existing `total_ms` method body (line 82–84):

Old:
```rust
    pub fn total_ms(&self) -> f32 {
        self.pass_ms.iter().sum()
    }
```

New — replace with `frame_wall_ms` and keep a `total_ms` shim that compiles but panics at runtime so callers are forced to migrate (they are all in `app.rs`, addressed in Task 3):

```rust
    /// True whole-frame GPU wall-clock: `max(end_ticks) − min(begin_ticks)`.
    /// On Apple TBDR, pass pipelining means the sum of per-pass deltas overcounts
    /// by ~3×. Use this instead of `total_ms()` for any reported GPU total.
    ///
    /// Returns 0.0 when no valid timestamp pair has been read back yet (same
    /// zero-before-first-readback semantics as `pass_ms`).
    pub fn frame_wall_ms_from_buf(&self) -> f32 {
        // Re-derive the raw ticks from pass_ms is not possible (pass_ms is
        // already per-pass f32). We cache the last resolved wall-clock instead.
        // See `after_submit` where this field is populated.
        self.wall_ms
    }
```

Wait — `pass_ms` stores f32 deltas, not raw ticks; the raw ticks are only available during the `after_submit` readback window. To expose the wall-clock we need to cache it. Update the struct to store `wall_ms: f32` alongside `pass_ms`:

In `git-craft/src/render/timestamps.rs`, update `GpuTimer` struct: add `pub wall_ms: f32` after `pub pass_ms: Vec<f32>`:

```rust
    /// True whole-frame GPU wall-clock (see `frame_wall_ms`). Updated each
    /// time `after_submit` completes a successful readback. Zero before the
    /// first readback.
    pub wall_ms: f32,
```

In `GpuTimer::new`, add `wall_ms: 0.0` to the constructor `Self { ... }` block (after the `pass_ms` line):

```rust
            pass_ms: vec![0.0; labels.len()],
            wall_ms: 0.0,
```

In `after_submit`, inside the `if self.map_ok.load(...)` block (after the `for (i, ms) in pass_millis(...)` loop), add:

```rust
                    if let Some(w) = frame_wall_ms(ticks, period) {
                        self.wall_ms = w;
                    }
```

Remove the old `total_ms` method entirely. Add this method:

```rust
    /// True whole-frame GPU wall-clock in ms. Derived from the raw timestamp
    /// buffer on each successful readback. Zero before the first readback.
    ///
    /// On Apple TBDR, consecutive render passes pipeline; summing per-pass deltas
    /// overcounts by ~3×. This is the accurate replacement.
    pub fn total_ms(&self) -> f32 {
        self.wall_ms
    }
```

**Rationale for keeping the `total_ms` name:** it is called in three places in `app.rs` (lines 943, 1394) and having the right semantics under the same name minimises the diff and the blast radius. The `wall_ms` field is `pub` for tests.

- [ ] **Step 6: Run all tests to confirm no regressions**

```bash
cargo test 2>&1 | tail -20
```
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft/.worktrees/gpu-frametime-metric/git-craft
cargo fmt
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
cd /Users/onurtellioglu/Github/Minecraft/.worktrees/gpu-frametime-metric
git add git-craft/src/render/timestamps.rs
git commit -m "fix: replace per-pass-sum GPU metric with true wall-clock (frame_wall_ms)

On Apple TBDR, consecutive passes pipeline so summing per-pass timestamp
deltas overcounts real GPU frame time by ~3× (measured: 101 fps actual
but HUD showed 27.50 ms sum vs ~9.9 ms true). frame_wall_ms() uses
max(end_ticks) - min(begin_ticks) from the resolved timestamp buffer.
GpuTimer::total_ms() now returns the wall-clock instead of the sum.
Per-pass pass_ms breakdown is unchanged (still useful for hotspot work).

Refs #16"
git push
```

---

## Task 2: Extend `BenchConfig` with `native_res` flag

**Files:**
- Modify: `git-craft/src/bench.rs` (lines 20–43, 104–150, and tests section)

### Background

`BenchConfig` (line 20–23) currently holds only `frames: usize`. `parse_bench_args` (line 28–43) checks for `--bench` and `--bench-frames N`. We add `native_res: bool` toggled by `--native-bench`. When `native_res` is true, the window should be sized to the primary display's native pixels; `app.rs` reads this flag from the config.

`format_report` currently says "git-craft --bench" in its header; we extend it to print the resolution so the two modes are distinguishable in CI output.

- [ ] **Step 1: Write the failing tests**

Add to `bench.rs` `#[cfg(test)]` block (after the `parse_bench_args_detects_flag_and_frames` test):

```rust
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
        let result = parse_bench_args(
            ["prog", "--native-bench"].map(String::from).into_iter()
        );
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
        let cfg = parse_bench_args(
            ["prog", "--bench"].map(String::from).into_iter(),
        )
        .unwrap();
        assert!(!cfg.native_res);
    }

    #[test]
    fn format_report_includes_resolution_label() {
        let s = Summary { frames: 10, min: 1.0, mean: 2.0, p50: 2.0, p95: 3.0, p99: 4.0, max: 5.0 };
        let r = format_report(&s, &s, 120.0, false, 12, "1280×720");
        assert!(r.contains("1280×720"), "report should contain resolution: {r}");
    }
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test -p git-craft bench 2>&1 | tail -20
```
Expected: compile errors — `native_res` field and `format_report` signature mismatch.

- [ ] **Step 3: Implement `native_res` in `BenchConfig` and `parse_bench_args`**

In `git-craft/src/bench.rs`, update `BenchConfig`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BenchConfig {
    /// Number of frames to record after warmup.
    pub frames: usize,
    /// When true, the bench window is sized to the primary display's native
    /// physical pixel resolution instead of the hardcoded 1280×720.
    /// Activated by `--native-bench` (requires `--bench`).
    pub native_res: bool,
}
```

Update `parse_bench_args` to detect `--native-bench`:

```rust
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
```

- [ ] **Step 4: Update `format_report` signature to accept a `resolution` label**

`format_report` is called once in `app.rs` (line 965). We extend its signature with a `resolution: &str` parameter so the printed report says which resolution was measured. This is purely additive and doesn't change the verdict logic.

Replace `format_report` signature and header line:

```rust
pub fn format_report(
    cpu: &Summary,
    gpu: &Summary,
    target_fps: f32,
    timestamps: bool,
    render_radius: i32,
    resolution: &str,
) -> String {
    let budget_ms = 1000.0 / target_fps;
    let (metric, metric_p99) = if timestamps {
        ("GPU", gpu.p99)
    } else {
        ("CPU", cpu.p99)
    };
    let verdict = if metric_p99 <= budget_ms {
        "PASS"
    } else {
        "FAIL"
    };
    let mut s = String::new();
    s.push_str("=== git-craft --bench ===\n");
    s.push_str(&format!("resolution:       {resolution}\n"));
    s.push_str(&format!(
        "render distance:  {render_radius} columns ({}-chunk diameter, {} blocks)\n",
        render_radius * 2,
        render_radius * 32,
    ));
    // ... rest unchanged
```

The test `report_verdict_tracks_the_chosen_metric` in `bench.rs` will need to pass `"1280×720"` as the new final argument. Update those two `format_report` calls in the existing test.

- [ ] **Step 5: Run tests**

```bash
cargo test -p git-craft bench 2>&1 | tail -20
```
Expected: all tests pass. Fix any compile errors from the updated `format_report` signature.

- [ ] **Step 6: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft/.worktrees/gpu-frametime-metric
git add git-craft/src/bench.rs
git commit -m "feat: add --native-bench flag to BenchConfig (native_res field)

Extends BenchConfig with native_res: bool, set by --native-bench CLI
flag (requires --bench). format_report gains a resolution: &str param
so CI output identifies which resolution was benchmarked.
app.rs wiring comes in the next task.

Refs #16"
git push
```

---

## Task 3: Wire `--native-bench` into `app.rs` window creation

**Files:**
- Modify: `git-craft/src/app.rs` (lines 167, 943, 965, 1394, 1533–1540)

### Background

Three changes in `app.rs`:

1. **Window size branch** (line 1533–1540): when `native_res` is true, query `event_loop.primary_monitor()` for its native size and use that instead of `BENCH_WINDOW`. If the primary monitor cannot be queried (headless CI), fall back to `BENCH_WINDOW` with a log warning.

2. **`total_ms()` calls** (lines 943 and 1394): these already get the new wall-clock from Task 1 — no change needed in their call sites, but we must thread the resolution string into `format_report` (line 965).

3. **`format_report` call** (line 965): pass the actual window inner size as the resolution string.

- [ ] **Step 1: Update `App::new` to carry `native_res`**

`App::new` receives `bench_cfg: Option<BenchConfig>`. The `native_res` flag must survive until `resumed()` where the window is created. The cleanest approach: store the original `BenchConfig` instead of wrapping it immediately. Currently `App` has `bench: Option<BenchRun>` (line 254). Add a separate field:

```rust
    /// True when --native-bench was passed: window sized to primary display
    /// native pixels instead of BENCH_WINDOW.
    native_bench: bool,
```

In `App::new`, initialise it:

```rust
            native_bench: bench_cfg.map(|c| c.native_res).unwrap_or(false),
```

- [ ] **Step 2: Update `resumed()` window sizing**

Replace the bench window creation block (lines 1535–1540):

Old:
```rust
        if bench {
            attrs = attrs.with_title("git-craft (bench)").with_inner_size(
                winit::dpi::PhysicalSize::new(BENCH_WINDOW.0, BENCH_WINDOW.1),
            );
            self.day.time = BENCH_NOON;
        }
```

New:
```rust
        if bench {
            let (pw, ph) = if self.native_bench {
                // Size the window to the primary display's native physical pixels.
                // Falls back to BENCH_WINDOW if the monitor cannot be queried
                // (headless CI, virtual display, etc.).
                event_loop
                    .primary_monitor()
                    .map(|m| {
                        let s = m.size();
                        (s.width, s.height)
                    })
                    .unwrap_or_else(|| {
                        log::warn!(
                            "--native-bench: primary monitor unavailable, \
                             falling back to {}×{}",
                            BENCH_WINDOW.0, BENCH_WINDOW.1
                        );
                        BENCH_WINDOW
                    })
            } else {
                BENCH_WINDOW
            };
            let title = if self.native_bench {
                "git-craft (native-bench)"
            } else {
                "git-craft (bench)"
            };
            attrs = attrs
                .with_title(title)
                .with_inner_size(winit::dpi::PhysicalSize::new(pw, ph));
            self.day.time = BENCH_NOON;
        }
```

- [ ] **Step 3: Pass resolution string to `format_report`**

The `format_report` call is at line 965. We have access to `gpu.config.width` / `gpu.config.height` (the swapchain size) at that point. Replace:

Old:
```rust
                        let report = crate::bench::format_report(
                            &cpu,
                            &gpu,
                            BENCH_TARGET_FPS,
                            timestamps,
                            RENDER_RADIUS,
                        );
```

New:
```rust
                        let res_label = format!(
                            "{}×{}",
                            self.gpu.as_ref().map(|g| g.config.width).unwrap_or(BENCH_WINDOW.0),
                            self.gpu.as_ref().map(|g| g.config.height).unwrap_or(BENCH_WINDOW.1),
                        );
                        let report = crate::bench::format_report(
                            &cpu,
                            &gpu,
                            BENCH_TARGET_FPS,
                            timestamps,
                            RENDER_RADIUS,
                            &res_label,
                        );
```

- [ ] **Step 4: Build and check**

```bash
cd /Users/onurtellioglu/Github/Minecraft/.worktrees/gpu-frametime-metric/git-craft
cargo build --release 2>&1 | grep -E "^error" | head -20
```
Expected: no errors. Fix any compile errors.

- [ ] **Step 5: Run tests + lint**

```bash
cargo test 2>&1 | tail -20
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
cargo fmt
```
Expected: all pass, no warnings, no formatting diffs.

- [ ] **Step 6: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft/.worktrees/gpu-frametime-metric
git add git-craft/src/app.rs
git commit -m "feat: wire --native-bench into window creation (primary display resolution)

When --bench --native-bench is passed, the bench window is sized to the
primary monitor's native physical pixels via event_loop.primary_monitor().
Falls back to 1280x720 with a log warning when the monitor is unavailable
(headless CI). The report output now includes the resolution so 1280x720
and native runs are distinguishable. Closes #16.

Refs #13
Closes #16"
git push
```

---

## Task 4: CHANGELOG update

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add `[Unreleased]` entry**

In `CHANGELOG.md`, replace the empty `## [Unreleased]` line (line 11) with:

```markdown
## [Unreleased]

### Fixed

- **GPU time metric now reports true wall-clock, not sum of per-pass deltas.**
  On Apple TBDR (all Apple Silicon GPUs), consecutive render passes pipeline:
  pass N's fragment phase overlaps pass N+1's tiler phase. Summing per-pass
  timestamp deltas therefore overcounts real GPU frame time by ~2–3×
  (v0.6.1 measured: 27.50 ms HUD sum vs ~9.9 ms actual frame time at 101 fps).
  `GpuTimer::total_ms()` now returns `max(end_ticks) − min(begin_ticks)` from
  the resolved timestamp buffer, matching the true elapsed GPU wall-clock.
  Per-pass `pass_ms` breakdown in the F3 HUD is unchanged — per-pass values
  are still correct for identifying hotspot passes — but **they are NOT additive
  on TBDR; the GPU total is always less than their sum**.

### Added

- `--native-bench` flag: run the existing deterministic flythrough bench at the
  primary display's native physical pixel resolution (e.g. 3024×1964 on M4)
  instead of the hardcoded 1280×720. Requires `--bench`. The printed report now
  includes the resolution so multiple bench runs are distinguishable.
  This enables measuring the actual native-resolution GPU cost to track
  progress on the 120 fps / 8.33 ms budget goal (issue #13).
```

- [ ] **Step 2: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft/.worktrees/gpu-frametime-metric
git add CHANGELOG.md
git commit -m "docs: CHANGELOG [Unreleased] — wall-clock GPU metric + --native-bench

Refs #13
Refs #16"
git push
```

---

## Self-Review Checklist

**Spec coverage:**

| Requirement | Task |
|---|---|
| Fix GPU total = `last_pass_end − first_pass_begin` | Task 1 |
| Keep per-pass HUD breakdown unchanged | Task 1 (only `wall_ms` changes; `pass_ms` untouched) |
| TDD for the pure-function metric change | Task 1 (6 tests before implementation) |
| Bench GPU percentiles use wall-clock | Task 1 (`total_ms()` now returns `wall_ms`) |
| HUD `GPU ms:` total uses wall-clock | Task 3 (same `total_ms()` call at line 1394) |
| Add `--native-bench` flag | Tasks 2 + 3 |
| Default `--bench` (1280×720) unchanged | Task 3 (BENCH_WINDOW path preserved) |
| CHANGELOG `[Unreleased]` with TBDR note | Task 4 |
| `Refs #13` + `Closes #16` in final commit | Task 3 commit |

**No placeholders:** all code blocks are complete. All command lines include expected output.

**Type consistency:**
- `frame_wall_ms` function signature: `(ticks: &[u64], period_ns: f32) -> Option<f32>` — used in both unit tests (Task 1) and `after_submit` impl (Task 1).
- `GpuTimer::wall_ms: f32` field — initialised in `new()`, written in `after_submit`, read via `total_ms()`.
- `BenchConfig::native_res: bool` — read in `App::new` → stored as `App::native_bench: bool` → read in `resumed()`.
- `format_report(..., resolution: &str)` — updated in Task 2, called in Task 3.
