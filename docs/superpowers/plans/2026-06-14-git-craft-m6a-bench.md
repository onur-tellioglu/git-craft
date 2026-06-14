---
title: git-craft M6a — `--bench` harness + performance baseline
date: 2026-06-14
domain: app-harness
type: enhancement
priority: high
breaking: false
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files: [git-craft/src/main.rs, git-craft/src/bench.rs, git-craft/src/app.rs, git-craft/src/render/gpu.rs, git-craft/CHANGELOG.md, git-craft/AGENTS.md]
---

# git-craft M6a — `--bench` harness + performance baseline

**Goal:** Implement the `--bench` mode that AGENTS.md and the design spec (§10 M6) already
reference but which does not exist yet. `cargo run --release -- --bench` runs a deterministic,
fixed-route flythrough at the full 384-block render distance, records steady-state frame-time
and GPU-time samples, prints **percentiles** (min / mean / p50 / p95 / p99 / max) plus a
PASS/FAIL verdict against the 120 fps (8.33 ms) budget, then exits on its own. This is the
M6 performance baseline that the texture and persistence work will be measured against.

**Why a separate, deterministic mode:** the interactive loop is input-driven and vsync-capped
(`PresentMode::Fifo`), so frame-to-frame timing there only reports the display cadence, not GPU
cost. The bench must (a) be reproducible run-to-run (fixed seed `1337`, fixed window size, fixed
camera route, frozen day/night at noon), (b) measure steady state (warm up until streaming is
idle before recording), and (c) report a vsync-independent number — GPU pass time from the
existing `GpuTimer` is the primary metric; CPU frame time (under `Immediate` present mode when
available) is reported alongside.

**Architecture:**
1. **`src/bench.rs` — pure, unit-tested core.** No I/O, no wgpu. Holds:
   - `percentile(sorted, p)` and `summarize(samples) -> Option<Summary>` (min/mean/p50/p95/p99/max).
   - `bench_yaw(frame, total)` — the camera-orbit sweep (0 → 2π across the recorded window).
   - `format_report(cpu, gpu, target_fps, timestamps_available, render_radius)` — the printed text + verdict.
   - `BenchRun` — the warmup→record→done state machine over plain counters: `warmup_step(idle)`
     returns true once streaming has been idle for `STEADY_FRAMES` consecutive frames (or the
     warmup cap is hit), `push(cpu_ms, gpu_ms)` records a frame, `is_done()` ends the run.
2. **`main.rs` — arg parsing.** `--bench` enables it; `--bench-frames N` overrides the recorded
   frame count (default 600). Parsing is a pure helper `parse_bench_args(args) -> Option<BenchConfig>`
   (unit-tested), passed into `App::new`.
3. **`gpu.rs` — uncapped present in bench.** `Gpu::new` takes a `prefer_uncapped: bool`; when set
   and the surface advertises `PresentMode::Immediate`, use it (else fall back to `Fifo`). This makes
   CPU frame time meaningful; the GPU metric is vsync-independent regardless.
4. **`app.rs` — wiring.** `App` gains `bench: Option<BenchRun>`. In `resumed`, bench mode forces a
   fixed `1280×720` window and the uncapped present preference. In `render`, when bench is active:
   freeze the day cycle at noon, override the camera each frame from the orbit route at a fixed
   elevated vantage (so `update_world` streams the full radius around it), drive the state machine,
   and once done print the report and set an exit flag. `about_to_wait` performs the actual
   `event_loop.exit()` (it has the `ActiveEventLoop`; `render` does not).

**Validation:** `cargo test` covers the pure core (percentiles, summary, yaw route, report verdict,
warmup transition, arg parsing). `cargo run --release -- --bench` is run once to confirm it warms up,
records, prints the percentile block, and exits 0 — the printed numbers are the recorded M6 baseline
(captured in the PR description + CHANGELOG). Gates: `cargo fmt --check`, `cargo clippy --all-targets
-- -D warnings`, `cargo test`.

**Environment:** all `cargo` from `git-craft/`; `--release` only. Branch `feat/m6a-bench`; PR to `main`.

---

## Stage A — pure bench core (TDD)

- [x] **A1 `bench.rs` stats:** write tests first, then implement `percentile(sorted: &[f32], p: f32) -> f32`
  (linear-interpolated rank; clamps p to [0,1], handles len 0/1) and `summarize(samples: &[f32]) ->
  Option<Summary>` where `Summary { frames, min, mean, p50, p95, p99, max }` (None on empty). Tests:
  known sample set → exact p50/p95/p99; single sample → all equal; empty → None.
- [x] **A2 camera route:** `bench_yaw(frame: usize, total: usize) -> f32` sweeps `0 → TAU` linearly;
  `total == 0` and `frame >= total` are guarded. Tests: frame 0 → 0; frame total/2 → ~π; never panics.
- [x] **A3 state machine:** `BenchConfig { frames }`, `BenchRun::new(cfg)`, `warmup_step(idle: bool) ->
  bool` (counts consecutive idle frames, resets on a busy frame, returns true at `STEADY_FRAMES`
  consecutive or once `warmup_frames` exceeds `WARMUP_CAP`), `push(cpu_ms, gpu_ms)`, `recorded() ->
  usize`, `is_done()`. Tests: STEADY idles in a row → ready; a busy frame mid-streak resets the count;
  cap forces ready even if never idle; `push` × frames → `is_done()`.
- [x] **A4 report:** `format_report(cpu: &Summary, gpu: &Summary, target_fps: f32, timestamps: bool,
  render_radius: i32) -> String`. Verdict PASS when the chosen metric's p99 ≤ the frame budget
  (`1000/target_fps`); the metric is GPU p99 when `timestamps`, else CPU p99. Tests: a fast GPU
  summary → "PASS"; a slow one → "FAIL"; the no-timestamp path falls back to CPU and labels it.

## Stage B — CLI + GPU wiring

- [x] **B1 `main.rs` arg parse:** `parse_bench_args(args: impl Iterator<Item=String>) -> Option<BenchConfig>`
  (pure, unit-tested in `bench.rs`): returns `Some` when `--bench` is present, reads `--bench-frames N`
  (bad/missing N → default 600). `main` builds the `EventLoop`, calls it, and passes the result to
  `App::new(instance, bench_cfg)`.
- [x] **B2 `gpu.rs`:** `Gpu::new(instance, window, prefer_uncapped: bool)`; pick `Immediate` when
  preferred and present in `caps.present_modes`, else `Fifo`. Existing call sites pass `false`.

## Stage C — app integration + baseline

- [x] **C1 `app.rs` state:** `App::new(instance, bench_cfg: Option<BenchConfig>)` stores `bench:
  Option<BenchRun>`; add `should_exit: bool`. `resumed`: when bench, build the window with a fixed
  `1280×720` inner size and pass `prefer_uncapped = true` to `Gpu::new`; freeze `day.time = 0.25` (noon).
- [x] **C2 `app.rs` render hook:** at the top of `render` (after `dt`), if bench: skip `day.advance`;
  set the camera to the fixed vantage with `yaw = bench_yaw(recorded, frames)` and a fixed downward
  pitch. After `update_world`, compute `idle = jobs idle && upload_queue empty && columns_ready > 0`;
  while warming call `warmup_step(idle)`; once recording, `push(dt*1000, timer.total_ms())`. On
  `is_done()`: `println!` the report, set `should_exit = true`.
- [x] **C3 exit + baseline:** `about_to_wait`: if `should_exit`, `event_loop.exit()`. Run `cargo run
  --release -- --bench`; record the printed baseline in the PR + CHANGELOG `[Unreleased]`. Note the
  new flag in AGENTS.md if its description drifted. Commit: `feat: add --bench frame-time percentile
  harness and M6 perf baseline`.

---

## Self-Review
- Spec §10 M6 "performance pass validating 24 chunks @ 120 fps, `--bench` baseline" → fixed-route bench
  at the full `RENDER_RADIUS` (12 columns = 24-chunk diameter) reports percentiles + a 120 fps verdict. ✓
- Spec §11 "per-pass timestamps catch the offending pass early" → GPU metric reuses `GpuTimer`; the
  report can be extended to per-pass percentiles later without changing the harness shape.
- TBDR / pass order: bench changes no passes — it only drives the camera and reads timers. ✓
- Determinism: fixed seed, fixed window size, frozen noon, fixed orbit route, idle-gated warmup → the
  baseline is reproducible. The GPU-time metric is vsync-independent; CPU time uses `Immediate` when
  the surface offers it (documented fallback to `Fifo` otherwise).
- Engine-core discipline: all non-trivial logic lives in pure functions in `bench.rs` with tests first;
  `app.rs`/`gpu.rs` hold only the wgpu/winit glue, validated by the live `--bench` run.

