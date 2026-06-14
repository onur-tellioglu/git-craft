---
title: git-craft M5a — Shader Ladder Core (HDR, CSM, Hillaire Sky, Bloom + ACES)
date: 2026-06-12
domain: world-layer
type: enhancement
priority: high
breaking: false
db-migration: false
rls-affecting: false
slice: 5
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files:
  - git-craft/src/render/timestamps.rs
  - git-craft/src/render/targets.rs
  - git-craft/src/render/post.rs
  - git-craft/src/render/shadow.rs
  - git-craft/src/render/atmosphere.rs
  - git-craft/src/render/bloom.rs
  - git-craft/src/render/exposure.rs
  - git-craft/src/render/terrain.rs
  - git-craft/src/render/outline.rs
  - git-craft/src/render/hot_reload.rs
  - git-craft/src/render/mod.rs
  - git-craft/src/app.rs
  - git-craft/assets/shaders/terrain.wgsl
  - git-craft/assets/shaders/post.wgsl
  - git-craft/assets/shaders/shadow.wgsl
  - git-craft/assets/shaders/sky_luts.wgsl
  - git-craft/assets/shaders/sky.wgsl
  - git-craft/assets/shaders/bloom.wgsl
  - git-craft/assets/shaders/exposure.wgsl
trigger-tasks-touched: []
shared-modules-touched: []
---

# git-craft M5a — Shader Ladder Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The first four rungs of the M5 shader ladder — per-pass GPU timing + an HDR offscreen pipeline (rung 0, scaffold), 3-cascade PCF shadow maps with the spec §6 per-fragment lighting model (rung 1), a Hillaire physically based sky with LUT-driven sun/moon color and aerial-perspective fog (rung 2), and a 13-tap bloom chain with histogram auto-exposure and ACES tonemapping (rung 3).

**Architecture:** The frame stops rendering straight to the swapchain. New frame graph: `[compute: sky LUTs] → [render: shadow cascade 0..2, cadenced] → [render: opaque terrain + sky background + outline → HDR RGBA16F] → [render: bloom down/up chain] → [compute: exposure histogram + adaptation] → [render: post (exposure → bloom mix → ACES) → swapchain] → [egui]`. Terrain lighting moves from per-vertex to per-fragment so it can sample the CSM (spec §6 model: `direct·min(shadow, skylightGuard) + ambient + torch`, flood-fill skylight guarding caves beyond shadow range). Atmosphere is implemented twice from one constant set: a pure-Rust transmittance ray-marcher (unit-tested, drives sun/moon light color on the CPU) and the WGSL Hillaire LUTs (transmittance + multi-scatter computed on demand; sky-view + 32³ aerial-perspective froxels per frame). `GpuTimer` grows from one hardcoded pass to N labeled passes so the F3 HUD shows the spec §6 GPU budget table live. Everything stays within 4 bind groups (terrain uses exactly 4: frame / quads / shadow / aerial-LUT).

**Deferred to M5b (next plan, same branch):** GTAO + TAA + render-scale knob, froxel volumetrics, water SSR + refraction + transparent water pass. Depth/normal attachments stay `StoreOp::Discard`/absent until M5b needs them.

**Tech Stack:** Rust (edition 2024), wgpu 29, winit 0.30, glam 0.33, egui 0.34, bytemuck 1.25, naga (shader validation).

**Spec:** `docs/superpowers/specs/2026-06-11-dabcraft-design.md` §6 (frame graph, lighting model, TBDR discipline, GPU budget), §7 (day/night + moon), §8 (per-pass HUD timings), §9 (shader compile tests), §10 (M5), §11 (risks).

**No git remote exists** — skip all push/PR/issue steps (issue gate skipped for the same reason). Commit locally on branch `feat/m5-shaders` (created from `main` in Task 1; M5b continues on the same branch).

**Commit granularity:** the spec says "one rung per commit"; M4 shipped 13 commits and the subagent workflow amends per task. Resolution: commit per task (as in M1–M4); each commit message carries its rung in the subject suffix (`… (m5 rung N)`), and the last task of a rung states the rung is complete. The rung ordering itself is strict — a rung's tasks all land before the next rung starts.

**Environment:** every shell needs `export PATH="$HOME/.cargo/bin:$PATH"` before cargo commands. All commands run from the repo root (`~/Github/Minecraft`) with `--manifest-path git-craft/Cargo.toml`. macOS has no `timeout`; smoke tests use background-run + kill. `cargo fmt` is NOT a gate (waived deliberately; codebase is not rustfmt-formatted). Quality gates per task: `cargo test` + `cargo clippy --all-targets -- -D warnings`.

---

## Context primer (read before Task 1)

Key existing code you will build on (all paths under `git-craft/`):

- `render/terrain.rs` — `TerrainRenderer`: vertex-pulled quads from a storage-buffer arena (`quads` group 1: binding 0 quad array, binding 1 per-section origins), one `draw_indexed_indirect` per visible section (slot rides in `first_instance`), `FrameUniform { view_proj, sky, sun }` at group 0 binding 0 (96 bytes, layout-asserted by test `frame_uniform_layout_matches_wgsl`). `prepare()` frustum-culls and writes indirect args; `draw()` replays them. `swap_shader()` rebuilds the pipeline for hot-reload. **You will extend FrameUniform once (Task 5, final 208-byte layout) and add bind groups 2 (shadow) and 3 (aerial LUT).**
- `assets/shaders/terrain.wgsl` — all lighting currently per-vertex (`vs_main` computes final color). The `PALETTE` const table is parsed by the Rust test `block.rs::colors_match_the_shader_palette` — **keep the table text intact** when rewriting the shader.
- `render/timestamps.rs` — `GpuTimer` measures exactly one render pass (2 queries). Readback is async (a few frames latent) with a `pending` gate; that pattern is preserved when generalizing to N passes.
- `render/hot_reload.rs` — `validate_wgsl` (naga parse + validate, baseline capabilities) and `ShaderWatcher` (mtime poll, 500 ms). Test `shipped_terrain_shader_is_valid` will be generalized to a directory glob over `assets/shaders/*.wgsl` — after that, every new shader is automatically compile-tested (spec §9).
- `render/depth.rs` — `DEPTH_FORMAT = Depth32Float`, `create_depth_view`. Main-pass depth stays `StoreOp::Discard` in M5a.
- `render/outline.rs` — `OutlineRenderer::new(device, surface_format, src)`: its color target format must become the HDR format in Task 2 (it draws inside the main pass).
- `render/gpu.rs` — `Gpu { surface, device, queue, config }`. `config.format` is the sRGB swapchain format; writes to it are auto-encoded, so the post shader outputs **linear** color.
- `app.rs::render()` — the whole frame: hot-reload poll → sim → `terrain.write_frame` + `prepare` → one render pass (terrain + outline) → egui → submit. egui draws into the **swapchain** view and must stay last. `App::resumed` builds all renderers. `RENDER_RADIUS = 12` (384 blocks).
- `game/camera.rs` — `Camera { position, yaw, pitch, fov_y }`, `forward()`, `view_proj(aspect)` (perspective_rh, near 0.1, `FAR_PLANE = 800.0`, wgpu depth 0..1).
- `game/daycycle.rs` — `DayCycle::sun_dir()` (toward sun, rises +X, tilted +Z by 0.12 before normalize — **never parallel to ±Z**, so `Vec3::Z` is a safe light-space up vector), `day_factor()` ∈ [0.03, 1], `sky_color()` (linear). These stay; light *color* moves to the atmosphere module (Task 6).
- `render/frustum.rs` — `Frustum::from_view_proj(Mat4)` + `intersects_aabb`. Gribb-Hartmann plane extraction works for orthographic matrices too — reuse it for per-cascade shadow culling.
- Lighting data reaching the shader: each quad carries 4-bit skylight/blocklight (flood-fill values, baked at mesh time) + 2-bit AO per corner. The day/night cycle never touches flood-fill data (spec §4).
- wgpu 29 API notes: `PipelineLayoutDescriptor` has `immediate_size: 0`; color attachments have `depth_slice: None`; render/compute pass descriptors have `multiview_mask: None` / `timestamp_writes`; `depth_write_enabled`/`depth_compare` are `Option`; bind group layouts ≤ 4 (default limit — we use exactly 4 on terrain).
- TBDR discipline (spec §6): `LoadOp::Clear` wherever possible, `StoreOp::Discard` for anything not sampled later, few large passes. The sky is drawn **inside** the main pass (fullscreen triangle at depth 1.0, compare LessEqual, no depth write) so hardware HSR kills sky fragments behind terrain and no extra pass boundary is paid.

Commands:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --manifest-path git-craft/Cargo.toml                  # all tests
cargo test --manifest-path git-craft/Cargo.toml shadow::         # one module
cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
cargo run --release --manifest-path git-craft/Cargo.toml         # play (debug too slow)
```

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/render/timestamps.rs` | rewrite | N labeled GPU pass timers (render + compute), async readback |
| `src/render/targets.rs` | new | HDR color target + bloom mip chain textures, resize |
| `src/render/post.rs` | new | Final post pass: exposure → bloom mix → ACES → swapchain |
| `src/render/shadow.rs` | new | Cascade math (pure, TDD) + CSM depth-pass renderer + cadence |
| `src/render/atmosphere.rs` | new | Atmosphere constants + pure CPU transmittance/sun-color (TDD) + LUT compute pipelines + sky background pipeline |
| `src/render/bloom.rs` | new | 13-tap downsample / tent-upsample chain over bloom mips |
| `src/render/exposure.rs` | new | Log-luminance histogram + temporal adaptation compute |
| `src/render/terrain.rs` | modify | FrameUniform v2 (208 B), bind groups 2+3, shadow-pass indirect-args helper, accessors |
| `src/render/outline.rs` | modify | Color target format → HDR |
| `src/render/hot_reload.rs` | modify | `ShaderSet` multi-file watcher; glob compile test for all shaders |
| `src/render/mod.rs` | modify | Register new modules |
| `src/app.rs` | modify | New frame graph wiring, pass labels, HUD per-pass times |
| `assets/shaders/terrain.wgsl` | rewrite | Per-fragment spec §6 lighting: PCF CSM + skylight guard + ambient/torch + aerial fog |
| `assets/shaders/post.wgsl` | new | Fullscreen blit → (Task 10+) bloom mix, exposure, ACES |
| `assets/shaders/shadow.wgsl` | new | Depth-only quad-pulling vertex shader |
| `assets/shaders/sky_luts.wgsl` | new | Hillaire transmittance / multi-scatter / sky-view / aerial-perspective LUT compute |
| `assets/shaders/sky.wgsl` | new | Sky background fullscreen draw (sky-view LUT + sun disc) |
| `assets/shaders/bloom.wgsl` | new | 13-tap downsample + 3×3 tent upsample fragment shaders |
| `assets/shaders/exposure.wgsl` | new | Histogram build + exposure resolve compute |

## Rung map

| Rung | Tasks | Lands |
|---|---|---|
| 0 — scaffold | 1–2 | Multi-pass GPU timer; HDR target + post blit; multi-shader hot-reload; shader glob test |
| 1 — CSM | 3–5 | Cascade math; 3×2048² depth passes with cadence; per-fragment PCF lighting + skylight guard |
| 2 — Hillaire | 6–9 | CPU atmosphere + sun/moon color; LUT compute; sky background draw; aerial-perspective fog |
| 3 — bloom/ACES | 10–12 | Bloom chain; histogram auto-exposure; ACES tonemap + budget check |
| — | 13 | Final review, full gates, manual playtest checklist |

---

### Task 1: Multi-pass GPU timer (rung 0)

**Files:**
- Rewrite: `git-craft/src/render/timestamps.rs`
- Modify: `git-craft/src/app.rs` (timer construction, pass writes, HUD)

The M1 `GpuTimer` measures exactly one render pass. M5 needs one timing slot per pass (spec §8: per-pass GPU times in the HUD). Keep the proven async-readback skeleton (pending gate, map_done/map_ok atomics, invalid-sample guard); generalize to N labeled slots, 2 queries each.

- [ ] **Step 1: Create the branch**

```bash
git checkout -b feat/m5-shaders
```

- [ ] **Step 2: Write the failing tests**

Append to the bottom of `timestamps.rs` (the file has no test module yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_millis_converts_begin_end_pairs() {
        // Two passes: 1 ms and 3 ms at a 1 ns/tick period.
        let ms = pass_millis(&[0, 1_000_000, 2_000_000, 5_000_000], 1.0);
        assert_eq!(ms, vec![Some(1.0), Some(3.0)]);
    }

    #[test]
    fn invalid_samples_yield_none() {
        // end == begin (pass skipped / never written) and end < begin
        // (Metal glitch) must both be rejected, not wrap to huge values.
        let ms = pass_millis(&[5, 5, 9, 3], 1.0);
        assert_eq!(ms, vec![None, None]);
    }

    #[test]
    fn timestamp_period_scales_the_result() {
        let ms = pass_millis(&[0, 1000], 41.7);
        assert!((ms[0].unwrap() - 41.7e-3 * 1000.0 / 1e6 * 1e3).abs() < 1e-6);
        // 1000 ticks × 41.7 ns = 41.7 µs = 0.0417 ms
        assert!((ms[0].unwrap() - 0.0417).abs() < 1e-6);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path git-craft/Cargo.toml timestamps::`
Expected: FAIL — `pass_millis` not found.

- [ ] **Step 4: Rewrite `timestamps.rs`**

Replace the whole file with:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Convert raw timestamp ticks (one begin/end pair per pass) to per-pass
/// milliseconds. An invalid pair (end <= begin: pass skipped this frame, or
/// Metal's occasional bad sample) yields None instead of a wrapped giant.
pub fn pass_millis(ticks: &[u64], period_ns: f32) -> Vec<Option<f32>> {
    ticks
        .chunks_exact(2)
        .map(|p| (p[1] > p[0]).then(|| (p[1] - p[0]) as f32 * period_ns / 1_000_000.0))
        .collect()
}

/// Measures N labeled GPU passes via pass-boundary timestamps (2 queries per
/// pass). Readback is async: `pass_ms` lags a few frames behind. While a
/// readback is pending, all `*_writes` return None and `resolve` is a no-op,
/// so a frame is either fully timed or not timed at all.
///
/// A pass that does not run in a given frame leaves its previous ticks in the
/// query set; the diff then repeats the old reading, which is the right HUD
/// behavior for cadenced passes (far shadow cascades).
pub struct GpuTimer {
    query_set: Option<wgpu::QuerySet>,
    resolve_buffer: wgpu::Buffer,
    read_buffer: wgpu::Buffer,
    map_done: Arc<AtomicBool>,
    map_ok: Arc<AtomicBool>,
    pending: bool,
    labels: &'static [&'static str],
    /// Per-pass milliseconds, indexed like `labels()`. Invalid samples keep
    /// the previous value.
    pub pass_ms: Vec<f32>,
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, labels: &'static [&'static str]) -> Self {
        let enabled = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let count = (labels.len() * 2) as u32;
        let query_set = enabled.then(|| {
            device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count,
            })
        });
        let size = labels.len() as u64 * 16;
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts resolve"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts read"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            query_set,
            resolve_buffer,
            read_buffer,
            map_done: Arc::new(AtomicBool::new(false)),
            map_ok: Arc::new(AtomicBool::new(false)),
            pending: false,
            labels,
            pass_ms: vec![0.0; labels.len()],
        }
    }

    pub fn labels(&self) -> &'static [&'static str] {
        self.labels
    }

    pub fn total_ms(&self) -> f32 {
        self.pass_ms.iter().sum()
    }

    fn query_set_for(&self, pass: usize) -> Option<&wgpu::QuerySet> {
        debug_assert!(pass < self.labels.len());
        if self.pending {
            return None;
        }
        self.query_set.as_ref()
    }

    pub fn render_writes(&self, pass: usize) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.query_set_for(pass).map(|qs| wgpu::RenderPassTimestampWrites {
            query_set: qs,
            beginning_of_pass_write_index: Some((pass * 2) as u32),
            end_of_pass_write_index: Some((pass * 2 + 1) as u32),
        })
    }

    /// Begin-only / end-only writes: one timing slot spanning a chain of
    /// passes (first pass gets begin, last pass gets end — used by bloom).
    pub fn render_writes_begin(&self, pass: usize) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.query_set_for(pass).map(|qs| wgpu::RenderPassTimestampWrites {
            query_set: qs,
            beginning_of_pass_write_index: Some((pass * 2) as u32),
            end_of_pass_write_index: None,
        })
    }

    pub fn render_writes_end(&self, pass: usize) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.query_set_for(pass).map(|qs| wgpu::RenderPassTimestampWrites {
            query_set: qs,
            beginning_of_pass_write_index: None,
            end_of_pass_write_index: Some((pass * 2 + 1) as u32),
        })
    }

    pub fn compute_writes(&self, pass: usize) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        self.query_set_for(pass).map(|qs| wgpu::ComputePassTimestampWrites {
            query_set: qs,
            beginning_of_pass_write_index: Some((pass * 2) as u32),
            end_of_pass_write_index: Some((pass * 2 + 1) as u32),
        })
    }

    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.pending {
            return;
        }
        if let Some(qs) = &self.query_set {
            let n = (self.labels.len() * 2) as u32;
            encoder.resolve_query_set(qs, 0..n, &self.resolve_buffer, 0);
            encoder.copy_buffer_to_buffer(
                &self.resolve_buffer,
                0,
                &self.read_buffer,
                0,
                self.labels.len() as u64 * 16,
            );
        }
    }

    pub fn after_submit(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.query_set.is_none() {
            return;
        }
        if !self.pending {
            let done = self.map_done.clone();
            let ok = self.map_ok.clone();
            self.read_buffer.map_async(wgpu::MapMode::Read, .., move |result| {
                // Signal completion even on failure, or `pending` locks forever
                // and the timer silently stops measuring.
                ok.store(result.is_ok(), Ordering::Release);
                done.store(true, Ordering::Release);
            });
            self.pending = true;
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        if self.map_done.swap(false, Ordering::AcqRel) {
            if self.map_ok.load(Ordering::Acquire) {
                {
                    let data = self.read_buffer.get_mapped_range(..);
                    let ticks: &[u64] = bytemuck::cast_slice(&data);
                    let period = queue.get_timestamp_period();
                    for (i, ms) in pass_millis(ticks, period).into_iter().enumerate() {
                        if let Some(ms) = ms {
                            self.pass_ms[i] = ms;
                        }
                    }
                }
                self.read_buffer.unmap();
            } else {
                // Failed map leaves the buffer unmapped; just retry next frame.
                log::warn!("timestamp readback map failed; retrying");
            }
            self.pending = false;
        }
    }
}
```

(then the test module from Step 2 at the bottom).

- [ ] **Step 5: Wire `app.rs` to the new API**

Near the other consts at the top of `app.rs` add the pass table. It grows in later tasks; Task 1 only has the main pass:

```rust
/// GPU pass timing slots (spec §8). Order is frame order; indices are stable
/// within a task but renumbered as the frame graph grows through M5.
const PASS_LABELS: &[&str] = &["main"];
const PASS_MAIN: usize = 0;
```

In `resumed()`:

```rust
self.timer = Some(GpuTimer::new(&gpu.device, PASS_LABELS));
```

In `render()` replace the old single-pass capture:

```rust
let ts_writes = self.timer.as_ref().and_then(|t| t.render_writes(PASS_MAIN));
```

In the HUD closure, replace the single `GPU ms:` label. First change the capture variable (before the closure):

```rust
let pass_ms: Vec<(&str, f32)> = self
    .timer
    .as_ref()
    .map(|t| t.labels().iter().copied().zip(t.pass_ms.iter().copied()).collect())
    .unwrap_or_default();
let gpu_total = self.timer.as_ref().map(|t| t.total_ms()).unwrap_or(0.0);
```

and inside the HUD window, where `GPU ms` was printed:

```rust
ui.label(format!("GPU ms:   {gpu_total:.2}"));
for (label, ms) in &pass_ms {
    ui.label(format!("  {label:<9} {ms:.2}"));
}
```

(remove the old `gpu_ms` capture variable).

- [ ] **Step 6: Run tests and clippy**

Run: `cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings`
Expected: all tests PASS (3 new in `timestamps::tests`), clippy clean.

- [ ] **Step 7: Smoke-run**

```bash
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 20 && kill $APP_PID
```
Expected: window opens, HUD shows `main` pass time ≈ previous GPU ms reading, no panics in the log.

- [ ] **Step 8: Commit**

```bash
git add git-craft/src/render/timestamps.rs git-craft/src/app.rs
git commit -m "feat: time GPU passes with a labeled multi-pass timer (m5 rung 0)"
```

---

### Task 2: HDR offscreen target + post pass + multi-shader hot-reload (completes rung 0)

**Files:**
- Create: `git-craft/src/render/targets.rs`
- Create: `git-craft/src/render/post.rs`
- Create: `git-craft/assets/shaders/post.wgsl`
- Modify: `git-craft/src/render/hot_reload.rs` (ShaderSet + glob test)
- Modify: `git-craft/src/render/mod.rs`, `git-craft/src/app.rs`

The main pass stops rendering to the swapchain: terrain + outline draw into an RGBA16F HDR texture, and a new post pass blits it to the swapchain with a fullscreen triangle (the rung-3 tonemap lands in this same shader later). The swapchain format is sRGB, so the post shader outputs linear and the hardware encodes. Hot-reload generalizes from one hardcoded watcher to a named set.

- [ ] **Step 1: Write the failing tests**

In `hot_reload.rs` tests, REPLACE `shipped_terrain_shader_is_valid` with a glob version (spec §9: every WGSL must compile; from now on each new shader is covered automatically):

```rust
    #[test]
    fn all_shipped_shaders_are_valid() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "wgsl") {
                let src = std::fs::read_to_string(&path).unwrap();
                if let Err(e) = validate_wgsl(&src) {
                    panic!("{} failed validation:\n{e}", path.display());
                }
                checked += 1;
            }
        }
        // terrain + outline + post at minimum; grows every rung.
        assert!(checked >= 3, "expected >= 3 shaders, found {checked}");
    }
```

- [ ] **Step 2: Run tests to verify the failure**

Run: `cargo test --manifest-path git-craft/Cargo.toml hot_reload::`
Expected: FAIL — `checked` is 2 (post.wgsl doesn't exist yet).

- [ ] **Step 3: Create `assets/shaders/post.wgsl`**

```wgsl
// Final post pass: HDR offscreen target -> swapchain.
// Rung 0: plain blit. Rung 3 adds bloom mix, auto-exposure, and ACES here.
// The swapchain view is sRGB; this shader outputs LINEAR color and the
// hardware encodes on write.

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var hdr_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Fullscreen triangle: UVs (0,0) (2,0) (0,2) cover the screen once.
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let hdr = textureSampleLevel(hdr_tex, hdr_samp, in.uv, 0.0).rgb;
    return vec4(hdr, 1.0);
}
```

- [ ] **Step 4: Create `src/render/targets.rs`**

```rust
/// Offscreen render targets, recreated on resize. M5a: the HDR color target.
/// The bloom mip chain joins in the bloom task; GTAO/normals arrive in M5b.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct RenderTargets {
    pub hdr_view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl RenderTargets {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let hdr = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr color"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self { hdr_view: hdr.create_view(&wgpu::TextureViewDescriptor::default()), width, height }
    }
}
```

- [ ] **Step 5: Create `src/render/post.rs`**

```rust
/// Fullscreen post pass: samples the HDR target, writes the swapchain.
pub struct PostPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    surface_format: wgpu::TextureFormat,
}

impl PostPass {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        hdr_view: &wgpu::TextureView,
        shader_source: &str,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let bind_group = Self::build_bind_group(device, &layout, hdr_view, &sampler);
        let pipeline = Self::build_pipeline(device, surface_format, &layout, shader_source);
        Self { pipeline, layout, bind_group, sampler, surface_format }
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(hdr_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    fn build_pipeline(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Rebind after the HDR target was recreated (resize).
    pub fn set_input(&mut self, device: &wgpu::Device, hdr_view: &wgpu::TextureView) {
        self.bind_group = Self::build_bind_group(device, &self.layout, hdr_view, &self.sampler);
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.pipeline = Self::build_pipeline(device, self.surface_format, &self.layout, shader_source);
    }

    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
}
```

- [ ] **Step 6: Add `ShaderSet` to `hot_reload.rs`**

Below `ShaderWatcher`:

```rust
/// Watches several named shader files; `poll()` returns every (name, source)
/// that changed and validated since the last call. Each inner watcher keeps
/// its own 500 ms poll throttle.
#[derive(Default)]
pub struct ShaderSet {
    watchers: Vec<(&'static str, ShaderWatcher)>,
}

impl ShaderSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn watch(&mut self, name: &'static str, path: impl Into<PathBuf>) {
        self.watchers.push((name, ShaderWatcher::new(path)));
    }

    pub fn poll(&mut self) -> Vec<(&'static str, String)> {
        self.watchers
            .iter_mut()
            .filter_map(|(name, w)| w.poll().map(|src| (*name, src)))
            .collect()
    }
}
```

- [ ] **Step 7: Register modules in `src/render/mod.rs`**

Add `pub mod targets;` and `pub mod post;` alongside the existing entries.

- [ ] **Step 8: Wire `app.rs`**

1. Fields: replace `shader_watcher: Option<crate::render::hot_reload::ShaderWatcher>` with `shaders: Option<crate::render::hot_reload::ShaderSet>`; add `targets: Option<crate::render::targets::RenderTargets>` and `post: Option<crate::render::post::PostPass>`. Update `App::new` accordingly (`shaders: None, targets: None, post: None`).

2. Add a shader-path helper near the consts:

```rust
fn shader_path(name: &str) -> String {
    format!("{}/assets/shaders/{name}", env!("CARGO_MANIFEST_DIR"))
}
```

3. In `resumed()` — watcher set FIRST (baseline mtimes must predate the source reads), then targets, then pipelines. Replace the old watcher/terrain/outline block with:

```rust
let mut shaders = crate::render::hot_reload::ShaderSet::new();
shaders.watch("terrain", shader_path("terrain.wgsl"));
shaders.watch("outline", shader_path("outline.wgsl"));
shaders.watch("post", shader_path("post.wgsl"));
self.shaders = Some(shaders);

let size = window.inner_size();
let targets = crate::render::targets::RenderTargets::new(&gpu.device, size.width, size.height);

let hdr_format = crate::render::targets::HDR_FORMAT;
let terrain_src = std::fs::read_to_string(shader_path("terrain.wgsl")).expect("terrain.wgsl missing");
self.terrain = Some(TerrainRenderer::new(&gpu.device, hdr_format, &terrain_src));

let outline_src = std::fs::read_to_string(shader_path("outline.wgsl")).expect("outline.wgsl missing");
self.outline = Some(crate::render::outline::OutlineRenderer::new(&gpu.device, hdr_format, &outline_src));

let post_src = std::fs::read_to_string(shader_path("post.wgsl")).expect("post.wgsl missing");
self.post = Some(crate::render::post::PostPass::new(&gpu.device, gpu.config.format, &targets.hdr_view, &post_src));
self.targets = Some(targets);
```

(`TerrainRenderer` and `OutlineRenderer` already take their target format as a parameter — passing `HDR_FORMAT` is the whole change; the egui layer keeps `gpu.config.format`.)

4. In `render()`, replace the single-watcher poll with:

```rust
if let (Some(set), Some(gpu)) = (self.shaders.as_mut(), self.gpu.as_ref()) {
    for (name, source) in set.poll() {
        match name {
            "terrain" => {
                if let Some(t) = self.terrain.as_mut() {
                    t.swap_shader(&gpu.device, &source);
                }
            }
            "post" => {
                if let Some(p) = self.post.as_mut() {
                    p.swap_shader(&gpu.device, &source);
                }
            }
            // outline has no swap_shader yet; restart to pick it up.
            _ => {}
        }
    }
}
```

5. In the frame: the main pass's color attachment view becomes the HDR target, and the post pass runs after it. Replace `view: &view` in the main pass with `view: hdr_view` where, before `encoder` is created:

```rust
let Some(targets) = self.targets.as_ref() else { return };
let hdr_view = &targets.hdr_view;
```

The main pass keeps its sky-color clear and `PASS_MAIN` timestamps. After the main pass block (and before the timer `resolve`), add:

```rust
let post_writes = self.timer.as_ref().and_then(|t| t.render_writes(PASS_POST));
if let Some(post) = self.post.as_ref() {
    post.draw(&mut encoder, &view, post_writes);
}
```

egui keeps drawing into `&view` (the swapchain) afterwards — unchanged.

6. Pass labels:

```rust
const PASS_LABELS: &[&str] = &["main", "post"];
const PASS_MAIN: usize = 0;
const PASS_POST: usize = 1;
```

7. Resize (`WindowEvent::Resized`): after recreating the depth view, recreate targets and rebind post:

```rust
self.targets = Some(crate::render::targets::RenderTargets::new(&gpu.device, size.width, size.height));
if let (Some(post), Some(targets)) = (self.post.as_mut(), self.targets.as_ref()) {
    post.set_input(&gpu.device, &targets.hdr_view);
}
```

- [ ] **Step 9: Run tests and clippy**

Run: `cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings`
Expected: PASS, including `all_shipped_shaders_are_valid` (3 shaders found).

- [ ] **Step 10: Smoke-run**

```bash
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 20 && kill $APP_PID
```
Expected: image identical to before (blit is a no-op visually), HUD now lists `main` and `post` pass times. Resize the window during the run — no crash, image stays correct.

- [ ] **Step 11: Commit (rung 0 complete)**

```bash
git add git-craft/src/render/targets.rs git-craft/src/render/post.rs git-craft/assets/shaders/post.wgsl \
        git-craft/src/render/hot_reload.rs git-craft/src/render/mod.rs git-craft/src/app.rs
git commit -m "feat: render through an HDR offscreen target with a post pass (m5 rung 0)"
```

---

### Task 3: Cascade split and light-matrix math (rung 1)

**Files:**
- Create: `git-craft/src/render/shadow.rs` (pure math half; the GPU half is Task 4)
- Modify: `git-craft/src/render/mod.rs`

Pure functions, TDD. Fitting strategy: each cascade wraps its frustum slice's bounding **sphere** (radius is rotation-invariant, so the ortho box never changes size as the camera turns), and the light matrix's XY translation is snapped to shadow-map texels (so a moving camera doesn't make shadow edges shimmer). Light-space up is `Vec3::Z` — safe because `DayCycle::sun_dir()` always carries a fixed +Z tilt (≈0.119 after normalize) and is therefore never parallel to ±Z, for the moon (`-sun_dir`) too.

- [ ] **Step 1: Write the failing tests**

Create `src/render/shadow.rs` containing only a test module for now:

```rust
use glam::{Mat4, Vec3};

#[cfg(test)]
mod tests {
    use super::*;

    const RES: u32 = 2048;

    fn light() -> Vec3 {
        Vec3::new(1.0, 1.0, 0.12).normalize()
    }

    #[test]
    fn splits_cover_the_range_monotonically() {
        let s = cascade_splits(0.5, 360.0, 0.7);
        assert_eq!(s[0], 0.5);
        assert_eq!(s[CASCADE_COUNT], 360.0);
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
            let texels = t.x * RES as f32 / 2.0;
            assert!((texels - texels.round()).abs() < 1e-2, "origin off-grid by {} texels", texels - texels.round());
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
```

Register the module: add `pub mod shadow;` to `src/render/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path git-craft/Cargo.toml shadow::`
Expected: FAIL — none of the functions exist.

- [ ] **Step 3: Implement the math**

Above the test module in `shadow.rs`:

```rust
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
        1 => frame % 2 == 0,
        _ => frame % 4 == 0,
    }
}
```

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test --manifest-path git-craft/Cargo.toml shadow:: && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings`
Expected: 7 tests PASS, clippy clean.

- [ ] **Step 5: Commit**

```bash
git add git-craft/src/render/shadow.rs git-craft/src/render/mod.rs
git commit -m "feat: add CSM split and texel-snapped light-matrix fitting (m5 rung 1)"
```

---

### Task 4: Shadow map renderer — three cascaded depth passes (rung 1)

**Files:**
- Modify: `git-craft/src/render/shadow.rs` (GPU half)
- Create: `git-craft/assets/shaders/shadow.wgsl`
- Modify: `git-craft/src/render/terrain.rs` (accessors + shadow indirect-args helper)
- Modify: `git-craft/src/app.rs`

The cascades render into a 3-layer 2048² `Depth32Float` array via a depth-only pipeline that reuses the terrain's quad/section storage buffers (group 1) with a per-cascade matrix in a dynamic-offset uniform (group 0). Terrain doesn't sample them until Task 5 — this task lands the passes and their HUD timings. Critical invariant: **a cascade's uniform matrix and its rendered depth content must always match**, so skipped (cadenced) cascades keep their cached `CascadeFit` and the terrain-facing uniform is written from the cache every frame.

- [ ] **Step 1: Write the failing layout test**

Append to `shadow.rs` tests:

```rust
    #[test]
    fn shadow_uniform_layout_matches_wgsl() {
        // 3 mat4 (192) + splits vec4 (16) + texels vec4 (16).
        assert_eq!(std::mem::size_of::<ShadowUniform>(), 224);
        assert_eq!(std::mem::offset_of!(ShadowUniform, splits), 192);
        assert_eq!(std::mem::offset_of!(ShadowUniform, texels), 208);
    }
```

Run: `cargo test --manifest-path git-craft/Cargo.toml shadow::` — FAIL (no `ShadowUniform`).

- [ ] **Step 2: Create `assets/shaders/shadow.wgsl`**

```wgsl
// Depth-only cascade pass: the same vertex pulling as terrain.wgsl with a
// minimal vertex stage and no fragment stage (spec §6 pass 2).

struct CascadeUniform {
    view_proj: mat4x4<f32>,
};

struct SectionInfo {
    origin: vec4<i32>,
};

@group(0) @binding(0) var<uniform> cascade: CascadeUniform;
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;
@group(1) @binding(1) var<storage, read> sections: array<SectionInfo>;

// Identical tables to terrain.wgsl (no WGSL includes; keep in sync by hand).
const FACE_ORIGIN = array<vec3<f32>, 6>(
    vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 0.0),
);
const FACE_U = array<vec3<f32>, 6>(
    vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0),
    vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0),
    vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0),
);
const FACE_V = array<vec3<f32>, 6>(
    vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0),
    vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0),
    vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0),
);
const CORNER_UV = array<vec2<f32>, 4>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> @builtin(position) vec4<f32> {
    let quad = quads[vi / 4u];
    let flip = extractBits(quad.y, 31u, 1u);
    let corner = (vi + flip) % 4u;
    let x = f32(extractBits(quad.x, 0u, 6u));
    let y = f32(extractBits(quad.x, 6u, 6u));
    let z = f32(extractBits(quad.x, 12u, 6u));
    let face = extractBits(quad.x, 18u, 3u);
    let w = f32(extractBits(quad.x, 21u, 5u) + 1u);
    let h = f32(extractBits(quad.y, 0u, 5u) + 1u);
    let uv = CORNER_UV[corner];
    let local = vec3(x, y, z) + FACE_ORIGIN[face] + FACE_U[face] * uv.x * w + FACE_V[face] * uv.y * h;
    let world = vec3<f32>(sections[slot].origin.xyz) + local;
    return cascade.view_proj * vec4(world, 1.0);
}
```

- [ ] **Step 3: Expose what the shadow pass needs from `TerrainRenderer`**

In `terrain.rs`: make the section-slot constant public (`pub const MAX_SECTIONS: u32 = 8192;`) and add accessors + the indirect-args helper:

```rust
    pub fn quads_layout(&self) -> &wgpu::BindGroupLayout {
        &self.quads_layout
    }

    pub fn quads_bind_group(&self) -> &wgpu::BindGroup {
        &self.quads_bind_group
    }

    pub fn index_buffer(&self) -> &wgpu::Buffer {
        &self.index_buffer
    }

    /// Write indirect args for every resident section intersecting `frustum`
    /// into `buffer` at `offset_bytes`; returns the draw count. Used by the
    /// shadow cascades — no cave culling (anything in the light frustum casts
    /// a shadow, seen or not).
    pub fn write_indirect_for(
        &self,
        queue: &wgpu::Queue,
        frustum: &Frustum,
        buffer: &wgpu::Buffer,
        offset_bytes: u64,
    ) -> u32 {
        let mut args: Vec<wgpu::util::DrawIndexedIndirectArgs> =
            Vec::with_capacity(self.entries.len());
        for (pos, e) in &self.entries {
            let min = pos.origin().as_vec3();
            if !frustum.intersects_aabb(min, min + glam::Vec3::splat(32.0)) {
                continue;
            }
            args.push(section_draw_args(e.offset, e.len, e.slot));
        }
        if !args.is_empty() {
            queue.write_buffer(buffer, offset_bytes, bytemuck::cast_slice(&args));
        }
        args.len() as u32
    }
```

Also note: the quads bind group's storage buffers are declared `visibility: wgpu::ShaderStages::VERTEX` — the shadow VS is also a vertex stage, so the existing layout works unchanged. (Task 5 extends the **camera** layout's visibility instead.)

- [ ] **Step 4: Implement `ShadowRenderer` in `shadow.rs`**

Add below the math half (one 256-byte dynamic-offset slot per cascade; 256 is the universal `min_uniform_buffer_offset_alignment`):

```rust
use crate::render::depth::DEPTH_FORMAT;
use crate::render::frustum::Frustum;
use crate::render::terrain::{TerrainRenderer, MAX_SECTIONS};
use crate::render::timestamps::GpuTimer;

/// Terrain-facing uniform: sampled by terrain.wgsl's fragment stage (Task 5).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowUniform {
    pub mats: [[[f32; 4]; 4]; CASCADE_COUNT],
    /// xyz = cascade far view-distances; w unused.
    pub splits: [f32; 4],
    /// xyz = world texel size per cascade (normal-offset scale); w unused.
    pub texels: [f32; 4],
}

const CASCADE_SLOT: u64 = 256; // dynamic-offset alignment
const INDIRECT_STRIDE: u64 = MAX_SECTIONS as u64 * 20;

pub struct ShadowRenderer {
    pipeline: wgpu::RenderPipeline,
    cascade_layout: wgpu::BindGroupLayout,
    quads_layout_id: (), // (layout is borrowed at build time only)
    cascade_buffer: wgpu::Buffer,
    cascade_bind_group: wgpu::BindGroup,
    layer_views: Vec<wgpu::TextureView>,
    array_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    indirect_buffer: wgpu::Buffer,
    draw_counts: [u32; CASCADE_COUNT],
    fits: [CascadeFit; CASCADE_COUNT],
    due: [bool; CASCADE_COUNT],
    frame: u64,
}

impl ShadowRenderer {
    pub fn new(
        device: &wgpu::Device,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> Self {
        let cascade_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cascade"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let cascade_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cascade mats"),
            size: CASCADE_SLOT * CASCADE_COUNT as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cascade_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cascade"),
            layout: &cascade_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &cascade_buffer,
                    offset: 0,
                    size: Some(std::num::NonZeroU64::new(64).unwrap()), // one mat4
                }),
            }],
        });

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow cascades"),
            size: wgpu::Extent3d {
                width: SHADOW_RESOLUTION,
                height: SHADOW_RESOLUTION,
                depth_or_array_layers: CASCADE_COUNT as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let layer_views = (0..CASCADE_COUNT as u32)
            .map(|i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("shadow layer"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow uniform"),
            size: std::mem::size_of::<ShadowUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let indirect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow indirect"),
            size: INDIRECT_STRIDE * CASCADE_COUNT as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline = Self::build_pipeline(device, &cascade_layout, quads_layout, shader_source);

        Self {
            pipeline,
            cascade_layout,
            quads_layout_id: (),
            cascade_buffer,
            cascade_bind_group,
            layer_views,
            array_view,
            uniform_buffer,
            indirect_buffer,
            draw_counts: [0; CASCADE_COUNT],
            fits: std::array::from_fn(|_| CascadeFit { view_proj: Mat4::IDENTITY, texel_world: 1.0 }),
            due: [false; CASCADE_COUNT],
            frame: 0,
        }
    }

    fn build_pipeline(
        device: &wgpu::Device,
        cascade_layout: &wgpu::BindGroupLayout,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(cascade_layout), Some(quads_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                // Slope-scaled bias against acne; the normal-offset in the
                // sampling shader (Task 5) handles the rest.
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None, // depth-only
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn swap_shader(
        &mut self,
        device: &wgpu::Device,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) {
        self.pipeline = Self::build_pipeline(device, &self.cascade_layout, quads_layout, shader_source);
    }

    pub fn uniform_buffer(&self) -> &wgpu::Buffer {
        &self.uniform_buffer
    }

    pub fn array_view(&self) -> &wgpu::TextureView {
        &self.array_view
    }

    /// Refit due cascades and write all GPU-visible state. Call once per
    /// frame BEFORE encoding (write_buffer lands before any submit).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        terrain: &TerrainRenderer,
        cam_pos: glam::Vec3,
        cam_forward: glam::Vec3,
        fov_y: f32,
        aspect: f32,
        light_dir: glam::Vec3,
    ) {
        self.frame += 1;
        let splits = cascade_splits(0.5, SHADOW_FAR, 0.7);
        for i in 0..CASCADE_COUNT {
            // First frame fits everything; afterwards the cadence rules.
            if self.frame > 1 && !cascade_due(self.frame, i) {
                continue;
            }
            let corners = slice_corners(cam_pos, cam_forward, fov_y, aspect, splits[i], splits[i + 1]);
            let fit = fit_light_matrix(&corners, light_dir, SHADOW_RESOLUTION);
            queue.write_buffer(
                &self.cascade_buffer,
                i as u64 * CASCADE_SLOT,
                bytemuck::bytes_of(&fit.view_proj.to_cols_array_2d()),
            );
            let frustum = Frustum::from_view_proj(fit.view_proj);
            self.draw_counts[i] =
                terrain.write_indirect_for(queue, &frustum, &self.indirect_buffer, i as u64 * INDIRECT_STRIDE);
            self.fits[i] = fit;
            self.due[i] = true;
        }
        // Terrain-facing uniform always reflects the CACHED fits, so sampling
        // matrices match the rendered maps even on skipped frames.
        let u = ShadowUniform {
            mats: std::array::from_fn(|i| self.fits[i].view_proj.to_cols_array_2d()),
            splits: [splits[1], splits[2], splits[3], 0.0],
            texels: [
                self.fits[0].texel_world,
                self.fits[1].texel_world,
                self.fits[2].texel_world,
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&u));
    }

    /// Encode the due cascade passes (consumes the `due` flags).
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        terrain: &TerrainRenderer,
        timer: Option<&GpuTimer>,
        first_pass_slot: usize,
    ) {
        for i in 0..CASCADE_COUNT {
            if !std::mem::take(&mut self.due[i]) {
                continue;
            }
            let ts = timer.and_then(|t| t.render_writes(first_pass_slot + i));
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.layer_views[i],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store, // sampled by terrain
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: ts,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.cascade_bind_group, &[(i as u64 * CASCADE_SLOT) as u32]);
            rpass.set_bind_group(1, terrain.quads_bind_group(), &[]);
            rpass.set_index_buffer(terrain.index_buffer().slice(..), wgpu::IndexFormat::Uint32);
            for d in 0..self.draw_counts[i] {
                rpass.draw_indexed_indirect(
                    &self.indirect_buffer,
                    i as u64 * INDIRECT_STRIDE + d as u64 * 20,
                );
            }
        }
    }
}
```

(Drop the `quads_layout_id: ()` placeholder field entirely if it survived into your version — it is shown only to flag that the quads layout is borrowed, never stored. The struct must not hold it.)

- [ ] **Step 5: Wire `app.rs`**

1. Field: `shadow: Option<crate::render::shadow::ShadowRenderer>` (+ `shadow: None` in `App::new`).
2. Pass labels become:

```rust
const PASS_LABELS: &[&str] = &["shadow0", "shadow1", "shadow2", "main", "post"];
const PASS_SHADOW0: usize = 0;
const PASS_MAIN: usize = 3;
const PASS_POST: usize = 4;
```

3. `resumed()`: watch the shader (`shaders.watch("shadow", shader_path("shadow.wgsl"));`) and build after terrain:

```rust
let shadow_src = std::fs::read_to_string(shader_path("shadow.wgsl")).expect("shadow.wgsl missing");
let terrain_ref = self.terrain.as_ref().unwrap();
self.shadow = Some(crate::render::shadow::ShadowRenderer::new(
    &gpu.device,
    terrain_ref.quads_layout(),
    &shadow_src,
));
```

4. Hot-reload match arm:

```rust
"shadow" => {
    if let (Some(s), Some(t)) = (self.shadow.as_mut(), self.terrain.as_ref()) {
        s.swap_shader(&gpu.device, t.quads_layout(), &source);
    }
}
```

5. In `render()` right after `terrain.prepare(...)` (still before `gpu.acquire()`):

```rust
let light_dir = self.day.sun_dir(); // Task 6 replaces with sun-or-moon
if let (Some(shadow), Some(terrain)) = (self.shadow.as_mut(), self.terrain.as_ref()) {
    shadow.prepare(
        &gpu.queue,
        terrain,
        self.camera.position,
        self.camera.forward(),
        self.camera.fov_y,
        aspect,
        light_dir,
    );
}
```

6. In the encoder, BEFORE the main pass block:

```rust
if let (Some(shadow), Some(terrain)) = (self.shadow.as_mut(), self.terrain.as_ref()) {
    shadow.encode(&mut encoder, terrain, self.timer.as_ref(), PASS_SHADOW0);
}
```

- [ ] **Step 6: Run tests and clippy**

Run: `cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings`
Expected: PASS (including the new `shadow_uniform_layout_matches_wgsl` and the shader glob test now covering `shadow.wgsl`), clippy clean.

- [ ] **Step 7: Smoke-run**

```bash
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 20 && kill $APP_PID
```
Expected: image unchanged (nothing samples the maps yet); HUD shows `shadow0` every frame and `shadow1`/`shadow2` ticking at their cadence; combined shadow time in the ~1 ms ballpark (spec budget 1.2 ms).

- [ ] **Step 8: Commit**

```bash
git add git-craft/src/render/shadow.rs git-craft/assets/shaders/shadow.wgsl \
        git-craft/src/render/terrain.rs git-craft/src/app.rs
git commit -m "feat: render three cadenced shadow-cascade depth passes (m5 rung 1)"
```

---

### Task 5: Per-fragment terrain lighting with PCF shadows (completes rung 1)

**Files:**
- Modify: `git-craft/src/render/terrain.rs` (FrameUniform v2, shadow bind group, FrameParams)
- Rewrite: `git-craft/assets/shaders/terrain.wgsl`
- Modify: `git-craft/src/app.rs`

Terrain lighting moves from the vertex to the fragment stage and becomes the spec §6 model: `albedo · (direct·min(shadow, skylightGuard) + ambient + torch)`. The `FrameUniform` is extended ONCE to its final M5a layout (208 bytes — includes the `inv_view_proj`/`camera`/`params` fields that Tasks 7–9 consume, so the struct never churns again). **Keep the `PALETTE` table text byte-identical** — `block.rs::colors_match_the_shader_palette` parses it.

- [ ] **Step 1: Update the failing layout test**

Replace `frame_uniform_layout_matches_wgsl` in `terrain.rs`:

```rust
    #[test]
    fn frame_uniform_layout_matches_wgsl() {
        // 2×mat4 (128) + 5×vec4 (80). WGSL would silently misread on drift.
        assert_eq!(std::mem::size_of::<FrameUniform>(), 208);
        assert_eq!(std::mem::offset_of!(FrameUniform, inv_view_proj), 64);
        assert_eq!(std::mem::offset_of!(FrameUniform, camera), 128);
        assert_eq!(std::mem::offset_of!(FrameUniform, sky), 144);
        assert_eq!(std::mem::offset_of!(FrameUniform, sun), 160);
        assert_eq!(std::mem::offset_of!(FrameUniform, sun_color), 176);
        assert_eq!(std::mem::offset_of!(FrameUniform, params), 192);
    }
```

Run: `cargo test --manifest-path git-craft/Cargo.toml terrain::` — FAIL.

- [ ] **Step 2: Extend `FrameUniform` and add `FrameParams`**

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    /// xyz = camera world position; w unused.
    camera: [f32; 4],
    /// rgb = ambient sky color (linear), w = day factor 0..1.
    sky: [f32; 4],
    /// xyz = light direction (toward sun or moon), w = 1 sun / 0 moon.
    sun: [f32; 4],
    /// rgb = light radiance after atmospheric transmittance.
    sun_color: [f32; 4],
    /// x,y = viewport px; z = aerial-perspective km per world meter (Task 9).
    params: [f32; 4],
}

/// Everything the frame uniform needs, gathered by the app.
pub struct FrameParams {
    pub view_proj: glam::Mat4,
    pub camera_pos: glam::Vec3,
    pub sky_color: glam::Vec3,
    pub day_factor: f32,
    pub light_dir: glam::Vec3,
    pub light_is_sun: bool,
    pub light_color: glam::Vec3,
    pub viewport: (u32, u32),
}
```

Replace `write_frame`:

```rust
    pub fn write_frame(&self, queue: &wgpu::Queue, p: &FrameParams) {
        let uniform = FrameUniform {
            view_proj: p.view_proj.to_cols_array_2d(),
            inv_view_proj: p.view_proj.inverse().to_cols_array_2d(),
            camera: [p.camera_pos.x, p.camera_pos.y, p.camera_pos.z, 0.0],
            sky: [p.sky_color.x, p.sky_color.y, p.sky_color.z, p.day_factor],
            sun: [p.light_dir.x, p.light_dir.y, p.light_dir.z, p.light_is_sun as u32 as f32],
            sun_color: [p.light_color.x, p.light_color.y, p.light_color.z, 0.0],
            params: [p.viewport.0 as f32, p.viewport.1 as f32, 0.0, 0.0], // z set in Task 9
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }
```

- [ ] **Step 3: Shadow bind group (group 2) on the terrain pipeline**

In `TerrainRenderer::new`, the camera layout's visibility becomes `wgpu::ShaderStages::VERTEX_FRAGMENT` (the FS reads `frame` now). Add a shadow layout + storage for the late-bound group:

```rust
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain shadow"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
```

New fields on `TerrainRenderer`: `shadow_layout: wgpu::BindGroupLayout`, `shadow_bind_group: Option<wgpu::BindGroup>`, `shadow_sampler: wgpu::Sampler` (comparison sampler, created once):

```rust
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow compare"),
            mag_filter: wgpu::FilterMode::Linear, // hardware 2×2 PCF per tap
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
```

`build_pipeline` gains the layout: `bind_group_layouts: &[Some(camera_layout), Some(quads_layout), Some(shadow_layout)]` (signature: pass `shadow_layout` through like the others; `swap_shader` forwards `&self.shadow_layout`).

Late binding (called once from the app after the `ShadowRenderer` exists; the pipeline layout is static so this is just resource wiring):

```rust
    pub fn attach_shadow(
        &mut self,
        device: &wgpu::Device,
        uniform: &wgpu::Buffer,
        map: &wgpu::TextureView,
    ) {
        self.shadow_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain shadow"),
            layout: &self.shadow_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(map) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.shadow_sampler) },
            ],
        }));
    }
```

In `draw()`, after the existing bind groups:

```rust
        let Some(shadow_bg) = &self.shadow_bind_group else {
            return; // not wired yet — one dark frame beats a validation panic
        };
        rpass.set_bind_group(2, shadow_bg, &[]);
```

(place the guard before any `set_*` call so the early return is clean).

- [ ] **Step 4: Rewrite `assets/shaders/terrain.wgsl`**

Full replacement (PALETTE/FACE tables byte-identical to the old file):

```wgsl
struct FrameUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,    // xyz camera world pos
    sky: vec4<f32>,       // rgb ambient sky color (linear), w day factor
    sun: vec4<f32>,       // xyz light dir (toward sun/moon), w 1=sun 0=moon
    sun_color: vec4<f32>, // rgb light radiance
    params: vec4<f32>,    // xy viewport px, z aerial km-per-meter
};

struct SectionInfo {
    origin: vec4<i32>,
};

struct ShadowUniform {
    mats: array<mat4x4<f32>, 3>,
    splits: vec4<f32>,  // cascade far view-distances
    texels: vec4<f32>,  // world texel size per cascade
};

@group(0) @binding(0) var<uniform> frame: FrameUniform;
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;
@group(1) @binding(1) var<storage, read> sections: array<SectionInfo>;
@group(2) @binding(0) var<uniform> shadow: ShadowUniform;
@group(2) @binding(1) var shadow_map: texture_depth_2d_array;
@group(2) @binding(2) var shadow_samp: sampler_comparison;

// Per-face: origin offset (added to voxel pos), U axis, V axis.
// Face order matches Rust: 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z.
const FACE_ORIGIN = array<vec3<f32>, 6>(
    vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 0.0),
);
const FACE_U = array<vec3<f32>, 6>(
    vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0),
    vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0),
    vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0),
);
const FACE_V = array<vec3<f32>, 6>(
    vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0),
    vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0),
    vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0),
);
const FACE_NORMAL = array<vec3<f32>, 6>(
    vec3(1.0, 0.0, 0.0), vec3(-1.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0), vec3(0.0, -1.0, 0.0),
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, -1.0),
);
// Ambient directional shade (the direct term has real NdotL now).
const FACE_SHADE = array<f32, 6>(0.8, 0.8, 1.0, 0.5, 0.6, 0.6);
const TORCH_COLOR = vec3(1.0, 0.62, 0.33);

// M2 palette indexed by the quad's texture field = block id;
// procedural textures replace this in M6.
const PALETTE = array<vec3<f32>, 13>(
    vec3(1.0, 0.0, 1.0),      //  0 air (never rendered; magenta = bug)
    vec3(0.35, 0.62, 0.22),   //  1 grass
    vec3(0.45, 0.32, 0.2),    //  2 dirt
    vec3(0.52, 0.52, 0.54),   //  3 stone
    vec3(0.86, 0.81, 0.58),   //  4 sand
    vec3(0.91, 0.93, 0.95),   //  5 snow grass
    vec3(0.19, 0.36, 0.68),   //  6 water (opaque until M5)
    vec3(0.42, 0.31, 0.19),   //  7 oak log
    vec3(0.23, 0.43, 0.14),   //  8 oak leaves
    vec3(0.32, 0.23, 0.14),   //  9 spruce log
    vec3(0.16, 0.3, 0.19),    // 10 spruce leaves
    vec3(0.27, 0.5, 0.21),    // 11 cactus
    vec3(0.95, 0.71, 0.3),    // 12 torch
);

const CORNER_UV = array<vec2<f32>, 4>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

const SHADOW_TEXEL: f32 = 1.0 / 2048.0;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) @interpolate(flat) face: u32,
    @location(2) ao: f32,
    // x = skylight, y = blocklight (constant across a greedy quad).
    @location(3) @interpolate(flat) light: vec2<f32>,
    @location(4) @interpolate(flat) albedo: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> VsOut {
    let quad = quads[vi / 4u];
    let flip = extractBits(quad.y, 31u, 1u);
    // AO diagonal flip: rotating the corner mapping by one turns the fixed
    // index pattern (0,1,2)(0,2,3) into triangles (1,2,3)(1,3,0) — the same
    // rectangle cut along the other diagonal.
    let corner = (vi + flip) % 4u;

    let x = f32(extractBits(quad.x, 0u, 6u));
    let y = f32(extractBits(quad.x, 6u, 6u));
    let z = f32(extractBits(quad.x, 12u, 6u));
    let face = extractBits(quad.x, 18u, 3u);
    let w = f32(extractBits(quad.x, 21u, 5u) + 1u);
    let h = f32(extractBits(quad.y, 0u, 5u) + 1u);
    let ao = f32(extractBits(quad.y, 5u + corner * 2u, 2u));
    let skylight = f32(extractBits(quad.y, 13u, 4u)) / 15.0;
    let blocklight = f32(extractBits(quad.y, 17u, 4u)) / 15.0;
    let tex = extractBits(quad.y, 21u, 10u);

    let uv = CORNER_UV[corner];
    let local = vec3(x, y, z) + FACE_ORIGIN[face] + FACE_U[face] * uv.x * w + FACE_V[face] * uv.y * h;
    let world = vec3<f32>(sections[slot].origin.xyz) + local;

    var out: VsOut;
    out.clip = frame.view_proj * vec4(world, 1.0);
    out.world_pos = world;
    out.face = face;
    out.ao = ao / 3.0;
    out.light = vec2(skylight, blocklight);
    out.albedo = PALETTE[min(tex, 12u)];
    return out;
}

// 3×3 PCF over the selected cascade; each tap is hardware 2×2 PCF.
fn shadow_factor(world_pos: vec3<f32>, normal: vec3<f32>, view_dist: f32) -> f32 {
    var c: u32 = 3u;
    if view_dist < shadow.splits.x { c = 0u; }
    else if view_dist < shadow.splits.y { c = 1u; }
    else if view_dist < shadow.splits.z { c = 2u; }
    if c == 3u {
        return 1.0; // beyond the cascades: the skylight guard rules alone
    }
    // Normal-offset bias scaled by this cascade's texel footprint.
    let pos = world_pos + normal * shadow.texels[c] * 1.5;
    let p = shadow.mats[c] * vec4(pos, 1.0);
    let uv = vec2(p.x, -p.y) * 0.5 + 0.5;
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 1.0;
    }
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let o = vec2(f32(dx), f32(dy)) * SHADOW_TEXEL;
            sum += textureSampleCompareLevel(shadow_map, shadow_samp, uv + o, c, p.z);
        }
    }
    return sum / 9.0;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let normal = FACE_NORMAL[in.face];
    let view_dist = length(in.world_pos - frame.camera.xyz);
    let ndotl = max(dot(normal, frame.sun.xyz), 0.0);

    // Flood-fill skylight gates the direct term beyond shadow range and
    // underground (spec §6): caves stay dark at noon, shafts of light need
    // actual sky exposure.
    let guard = smoothstep(0.0, 0.5, in.light.x);
    var shadow_f = 0.0;
    if ndotl > 0.0 && guard > 0.0 {
        shadow_f = shadow_factor(in.world_pos, normal, view_dist);
    }

    let ao = mix(0.35, 1.0, in.ao);
    let direct = frame.sun_color.rgb * ndotl * min(shadow_f, guard);
    let ambient = frame.sky.rgb * pow(in.light.x, 1.8) * FACE_SHADE[in.face] * ao;
    let torch = TORCH_COLOR * 1.4 * pow(in.light.y, 1.6) * FACE_SHADE[in.face] * ao;
    let color = in.albedo * (direct + ambient + torch);
    return vec4(color, 1.0);
}
```

- [ ] **Step 5: Wire `app.rs`**

1. Replace the `terrain.write_frame(...)` call:

```rust
let light_dir = self.day.sun_dir(); // Task 6: sun-or-moon + real radiance
let light_color = glam::Vec3::splat(2.2) * self.day.day_factor(); // placeholder until Task 6
terrain.write_frame(
    &gpu.queue,
    &crate::render::terrain::FrameParams {
        view_proj,
        camera_pos: self.camera.position,
        sky_color: self.day.sky_color(),
        day_factor: self.day.day_factor(),
        light_dir,
        light_is_sun: true,
        light_color,
        viewport: (gpu.config.width, gpu.config.height),
    },
);
```

(the `light_dir` binding moves up so the shadow `prepare` from Task 4 reuses it.)

2. In `resumed()`, after both terrain and shadow exist:

```rust
if let (Some(terrain), Some(shadow)) = (self.terrain.as_mut(), self.shadow.as_ref()) {
    terrain.attach_shadow(&gpu.device, shadow.uniform_buffer(), shadow.array_view());
}
```

- [ ] **Step 6: Run tests and clippy**

Run: `cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings`
Expected: PASS — including `block.rs::colors_match_the_shader_palette` (palette text unchanged) and the WGSL glob test on the rewritten terrain shader.

- [ ] **Step 7: Smoke-run and visual check**

```bash
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 30 && kill $APP_PID
```
Expected (eyeball + HUD): terrain casts real shadows (trees onto ground, mountains across valleys); shadow edges are soft (PCF) and don't crawl when moving; caves stay pitch dark at noon (guard); no acne on flat sunlit ground; `main` pass time roughly unchanged, `shadow*` ≈ 1 ms combined. Walk to a shadow boundary at dawn/dusk — no flicker between cascades.

- [ ] **Step 8: Commit (rung 1 complete)**

```bash
git add git-craft/src/render/terrain.rs git-craft/assets/shaders/terrain.wgsl git-craft/src/app.rs
git commit -m "feat: light terrain per-fragment with PCF cascaded shadows (m5 rung 1)"
```

---

### Task 6: CPU atmosphere — transmittance and sun/moon light color (rung 2)

**Files:**
- Create: `git-craft/src/render/atmosphere.rs` (pure CPU half; GPU halves are Tasks 7–9)
- Modify: `git-craft/src/render/mod.rs`, `git-craft/src/app.rs`

One parameter set, two implementations: this pure-Rust transmittance ray-marcher feeds the CPU (CSM light color, sunset reddening of the sun disc) and Task 7's WGSL mirrors it for the LUTs. Constants are Hillaire 2020's Earth values, distances in km, coefficients per km. TDD throughout.

- [ ] **Step 1: Write the failing tests**

Create `src/render/atmosphere.rs` with only the test module:

```rust
use glam::Vec3;

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Vec3) -> f32 {
        c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
    }

    /// DayCycle-style sun direction at day fraction t (0 = sunrise).
    fn sun_at(t: f32) -> Vec3 {
        let a = t * std::f32::consts::TAU;
        Vec3::new(a.cos(), a.sin(), 0.12).normalize()
    }

    #[test]
    fn zenith_transmittance_is_high_and_reddens_in_order() {
        let t = Atmosphere::default().transmittance(0.2, 1.0);
        // Rayleigh scatters blue hardest: r > g > b survive in that order.
        assert!(t.x > t.y && t.y > t.z, "{t}");
        assert!(t.x > 0.9, "red barely touched at zenith: {t}");
        assert!((0.6..0.9).contains(&t.z), "blue optical depth ~0.26: {t}");
    }

    #[test]
    fn horizon_sun_is_strongly_red() {
        let atm = Atmosphere::default();
        let h = atm.transmittance(0.2, 0.0);
        assert!(h.x > h.z * 20.0, "sunset must be red-dominant: {h}");
        assert!(h.z < 0.05, "blue should be nearly gone at the horizon: {h}");
    }

    #[test]
    fn below_horizon_from_ground_level_is_black() {
        assert_eq!(Atmosphere::default().transmittance(0.0001, -0.1), Vec3::ZERO);
    }

    #[test]
    fn transmittance_is_monotonic_in_elevation() {
        let atm = Atmosphere::default();
        let mut prev = Vec3::ZERO;
        for i in 0..=20 {
            let t = atm.transmittance(0.1, i as f32 / 20.0);
            assert!(t.x >= prev.x - 1e-4 && t.z >= prev.z - 1e-4, "mu step {i}: {t} < {prev}");
            prev = t;
        }
    }

    #[test]
    fn noon_light_is_the_sun_midnight_light_is_a_blue_moon() {
        let atm = Atmosphere::default();
        let (dir, color, is_sun) = dominant_light(&atm, sun_at(0.25), 0.1);
        assert!(is_sun && dir.y > 0.9, "noon: sun overhead");
        assert!(color.x >= color.z && lum(color) > 1.0, "noon sun is warm and strong: {color}");

        let (dir, color, is_sun) = dominant_light(&atm, sun_at(0.75), 0.1);
        assert!(!is_sun && dir.y > 0.9, "midnight: moon overhead (moon = -sun)");
        assert!(color.z > color.x, "moonlight is bluish: {color}");
        assert!(lum(color) < 0.05, "moonlight is weak: {color}");
    }

    #[test]
    fn light_never_pops_through_sunset() {
        let atm = Atmosphere::default();
        let mut prev = dominant_light(&atm, sun_at(0.47), 0.1).1;
        for i in 1..=60 {
            let t = 0.47 + 0.06 * i as f32 / 60.0; // sweep through sunset
            let c = dominant_light(&atm, sun_at(t), 0.1).1;
            assert!(lum(c - prev).abs() < 0.25, "luminance jump at t={t}: {prev} -> {c}");
            prev = c;
        }
    }
}
```

Register `pub mod atmosphere;` in `src/render/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path git-craft/Cargo.toml atmosphere::`
Expected: FAIL — nothing implemented.

- [ ] **Step 3: Implement**

Above the tests:

```rust
/// Earth-like atmosphere (Hillaire 2020 parameterization).
/// Distances in km, coefficients per km. The WGSL in sky_luts.wgsl mirrors
/// these constants — change BOTH or the sky stops matching the sun color.
pub struct Atmosphere {
    pub ground_radius: f32,
    pub top_radius: f32,
    pub rayleigh_scatter: Vec3,
    pub rayleigh_h: f32,
    pub mie_scatter: f32,
    pub mie_absorb: f32,
    pub mie_h: f32,
    pub ozone_absorb: Vec3,
    pub ozone_center: f32,
    pub ozone_half_width: f32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            ground_radius: 6360.0,
            top_radius: 6460.0,
            rayleigh_scatter: Vec3::new(5.802e-3, 13.558e-3, 33.1e-3),
            rayleigh_h: 8.0,
            mie_scatter: 3.996e-3,
            mie_absorb: 4.4e-3,
            mie_h: 1.2,
            ozone_absorb: Vec3::new(0.650e-3, 1.881e-3, 0.085e-3),
            ozone_center: 25.0,
            ozone_half_width: 15.0,
        }
    }
}

/// Top-of-atmosphere sun radiance. Tuned so the pre-exposure scene sits near
/// 1.0; auto-exposure (Task 11) makes the absolute scale uncritical.
pub const SUN_RADIANCE: Vec3 = Vec3::new(8.0, 7.6, 7.0);
/// Flat bluish moonlight (spec §7: "weak bluish directional moon light").
pub const MOON_RADIANCE: Vec3 = Vec3::new(0.012, 0.018, 0.032);

/// Nearest positive hit of the ray with a sphere of `radius` centered at the
/// planet origin; None when missed (or entirely behind the ray).
fn ray_sphere(origin: Vec3, dir: Vec3, radius: f32) -> Option<f32> {
    let b = origin.dot(dir);
    let c = origin.length_squared() - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    if -b - sq > 0.0 {
        Some(-b - sq)
    } else if -b + sq > 0.0 {
        Some(-b + sq)
    } else {
        None
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Atmosphere {
    fn extinction(&self, h: f32) -> Vec3 {
        let h = h.max(0.0);
        let rayl = (-h / self.rayleigh_h).exp();
        let mie = (-h / self.mie_h).exp();
        let ozone = (1.0 - (h - self.ozone_center).abs() / self.ozone_half_width).max(0.0);
        self.rayleigh_scatter * rayl
            + Vec3::splat((self.mie_scatter + self.mie_absorb) * mie)
            + self.ozone_absorb * ozone
    }

    /// Transmittance from `altitude` km above ground toward the sun at
    /// `cos_zenith`; zero when the ray hits the planet.
    pub fn transmittance(&self, altitude: f32, cos_zenith: f32) -> Vec3 {
        let pos = Vec3::new(0.0, self.ground_radius + altitude.max(1e-4), 0.0);
        let dir = Vec3::new((1.0 - cos_zenith * cos_zenith).max(0.0).sqrt(), cos_zenith, 0.0);
        if ray_sphere(pos, dir, self.ground_radius).is_some() {
            return Vec3::ZERO;
        }
        let Some(t_top) = ray_sphere(pos, dir, self.top_radius) else {
            return Vec3::ONE; // outside the atmosphere looking out
        };
        const STEPS: usize = 40;
        let dt = t_top / STEPS as f32;
        let mut depth = Vec3::ZERO;
        for i in 0..STEPS {
            let p = pos + dir * ((i as f32 + 0.5) * dt);
            depth += self.extinction(p.length() - self.ground_radius) * dt;
        }
        (-depth).exp()
    }
}

/// The frame's single analytic light: the transmittance-colored sun while it
/// is up, a weak bluish moon (direction -sun) otherwise. Both fade to ~zero
/// around the horizon, so the direction swap never pops visually.
/// Returns (direction toward the light, radiance, is_sun).
pub fn dominant_light(atm: &Atmosphere, sun_dir: Vec3, altitude_km: f32) -> (Vec3, Vec3, bool) {
    let sun_color = SUN_RADIANCE * atm.transmittance(altitude_km, sun_dir.y);
    let moon_dir = -sun_dir;
    let moon_color = MOON_RADIANCE * smoothstep(-0.04, 0.10, moon_dir.y);
    let lum = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    if lum(sun_color) >= lum(moon_color) {
        (sun_dir, sun_color, true)
    } else {
        (moon_dir, moon_color, false)
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path git-craft/Cargo.toml atmosphere::`
Expected: 6 tests PASS. If `horizon_sun_is_strongly_red` is off, check the ozone tent term and the 40-step march first — both dominate the horizon path.

- [ ] **Step 5: Wire `app.rs`**

Field `atmosphere: crate::render::atmosphere::Atmosphere` (init `Default::default()` in `App::new` — not an Option, it has no GPU deps). In `render()`, replace the Task 5 placeholder light:

```rust
let altitude_km = (self.camera.position.y / 1000.0).max(1e-4); // 1 block = 1 m
let (light_dir, light_color, light_is_sun) = crate::render::atmosphere::dominant_light(
    &self.atmosphere,
    self.day.sun_dir(),
    altitude_km,
);
```

and pass `light_is_sun` / `light_color` into `FrameParams` (the `shadow.prepare` call keeps taking `light_dir`).

- [ ] **Step 6: Run all tests, clippy, smoke-run**

```bash
cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 30 && kill $APP_PID
```
Expected: direct light visibly warms toward sunset and cools/weakens to bluish moonlight at night; moon shadows point opposite the daytime sun.

- [ ] **Step 7: Commit**

```bash
git add git-craft/src/render/atmosphere.rs git-craft/src/render/mod.rs git-craft/src/app.rs
git commit -m "feat: derive sun and moon light color from CPU atmospheric transmittance (m5 rung 2)"
```

---

### Task 7: Hillaire LUT compute passes (rung 2)

**Files:**
- Create: `git-craft/assets/shaders/sky_luts.wgsl`
- Modify: `git-craft/src/render/atmosphere.rs` (add `SkyLuts`), `git-craft/src/app.rs`

Three LUTs this task (the aerial-perspective froxel LUT is Task 9): transmittance 256×64 and multi-scatter 32×32 (computed once, and again after a hot-reload of the shader), sky-view 192×108 (every frame — it depends on sun direction and camera altitude). All `Rgba16Float`, written as storage textures, sampled with one linear clamp sampler (the sky-view's azimuth seam gets `AddressMode::Repeat` on U in Task 8's *sampling* of it — the LUT-internal sampler here stays clamp).

**Bind-group plan (usage-conflict-free):** group 0 = uniform + *sampled* LUT inputs + sampler; group 1 = exactly ONE storage output per pipeline. A texture may never appear as sampled input and storage output of the same dispatch, so each pipeline gets its own pair of explicit layouts covering exactly what its entry point uses (a layout may be a superset of what the shader uses, but a bind group must never contain a texture the dispatch also writes):

| pipeline | group 0 layout | group 1 layout |
|---|---|---|
| `cs_transmittance` | uniform(0) | storage `out_transmittance`(0) |
| `cs_multiscatter` | uniform(0), transmittance(1), sampler(3) | storage `out_multiscatter`(1) |
| `cs_skyview` | uniform(0), transmittance(1), multiscatter(2), sampler(3) | storage `out_skyview`(2) |

- [ ] **Step 1: Write the failing layout test**

Append to `atmosphere.rs` tests:

```rust
    #[test]
    fn atm_uniform_layout_matches_wgsl() {
        // mat4 (64) + camera vec4 + sun vec4 + radiance vec4.
        assert_eq!(std::mem::size_of::<AtmUniform>(), 112);
        assert_eq!(std::mem::offset_of!(AtmUniform, camera), 64);
        assert_eq!(std::mem::offset_of!(AtmUniform, sun), 80);
        assert_eq!(std::mem::offset_of!(AtmUniform, sun_radiance), 96);
    }
```

Run: `cargo test --manifest-path git-craft/Cargo.toml atmosphere::` — FAIL.

- [ ] **Step 2: Create `assets/shaders/sky_luts.wgsl`**

```wgsl
// Hillaire 2020 ("A Scalable and Production Ready Sky and Atmosphere
// Rendering Technique") LUT chain. Units: km; planet center at the origin;
// the camera sits on +Y at radius ground+altitude. Constants MUST mirror
// src/render/atmosphere.rs.

struct AtmUniform {
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,       // xyz world pos (meters), w altitude km
    sun: vec4<f32>,          // xyz toward the sun (world space)
    sun_radiance: vec4<f32>, // rgb top-of-atmosphere sun radiance
};

@group(0) @binding(0) var<uniform> atm: AtmUniform;
@group(0) @binding(1) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(2) var multiscatter_lut: texture_2d<f32>;
@group(0) @binding(3) var lut_samp: sampler;

@group(1) @binding(0) var out_transmittance: texture_storage_2d<rgba16float, write>;
@group(1) @binding(1) var out_multiscatter: texture_storage_2d<rgba16float, write>;
@group(1) @binding(2) var out_skyview: texture_storage_2d<rgba16float, write>;

const GROUND_R: f32 = 6360.0;
const TOP_R: f32 = 6460.0;
const RAYLEIGH_SCATTER = vec3(5.802e-3, 13.558e-3, 33.1e-3);
const RAYLEIGH_H: f32 = 8.0;
const MIE_SCATTER: f32 = 3.996e-3;
const MIE_ABSORB: f32 = 4.4e-3;
const MIE_H: f32 = 1.2;
const OZONE_ABSORB = vec3(0.650e-3, 1.881e-3, 0.085e-3);
const OZONE_CENTER: f32 = 25.0;
const OZONE_HALF_WIDTH: f32 = 15.0;
const PI: f32 = 3.14159265;

const TRANSMITTANCE_SIZE = vec2(256.0, 64.0);
const SKYVIEW_SIZE = vec2(192.0, 108.0);

struct Media {
    rayleigh: vec3<f32>,   // scattering
    mie: f32,              // scattering
    extinction: vec3<f32>,
};

fn media_at(h_in: f32) -> Media {
    let h = max(h_in, 0.0);
    let rayl = exp(-h / RAYLEIGH_H);
    let mie = exp(-h / MIE_H);
    let ozone = max(1.0 - abs(h - OZONE_CENTER) / OZONE_HALF_WIDTH, 0.0);
    var m: Media;
    m.rayleigh = RAYLEIGH_SCATTER * rayl;
    m.mie = MIE_SCATTER * mie;
    m.extinction = m.rayleigh + vec3((MIE_SCATTER + MIE_ABSORB) * mie) + OZONE_ABSORB * ozone;
    return m;
}

// Nearest positive hit with the origin-centered sphere; -1.0 when missed.
fn ray_sphere(origin: vec3<f32>, dir: vec3<f32>, radius: f32) -> f32 {
    let b = dot(origin, dir);
    let c = dot(origin, origin) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 { return -1.0; }
    let sq = sqrt(disc);
    if -b - sq > 0.0 { return -b - sq; }
    if -b + sq > 0.0 { return -b + sq; }
    return -1.0;
}

fn rayleigh_phase(c: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + c * c);
}

// Cornette-Shanks, g = 0.8.
fn mie_phase(c: f32) -> f32 {
    let g = 0.8;
    let g2 = g * g;
    return 3.0 * (1.0 - g2) * (1.0 + c * c)
        / (8.0 * PI * (2.0 + g2) * pow(1.0 + g2 - 2.0 * g * c, 1.5));
}

fn lut_uv(r: f32, mu: f32) -> vec2<f32> {
    return vec2(mu * 0.5 + 0.5, clamp((r - GROUND_R) / (TOP_R - GROUND_R), 0.0, 1.0));
}

fn sample_transmittance(r: f32, mu: f32) -> vec3<f32> {
    return textureSampleLevel(transmittance_lut, lut_samp, lut_uv(r, mu), 0.0).rgb;
}

fn sample_multiscatter(r: f32, mu_sun: f32) -> vec3<f32> {
    return textureSampleLevel(multiscatter_lut, lut_samp, lut_uv(r, mu_sun), 0.0).rgb;
}

fn march_transmittance(pos: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    if ray_sphere(pos, dir, GROUND_R) > 0.0 {
        return vec3(0.0);
    }
    let t_top = ray_sphere(pos, dir, TOP_R);
    if t_top <= 0.0 { return vec3(1.0); }
    let dt = t_top / 40.0;
    var depth = vec3(0.0);
    for (var i = 0u; i < 40u; i++) {
        let p = pos + dir * ((f32(i) + 0.5) * dt);
        depth += media_at(length(p) - GROUND_R).extinction * dt;
    }
    return exp(-depth);
}

@compute @workgroup_size(8, 8, 1)
fn cs_transmittance(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 256u || id.y >= 64u { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / TRANSMITTANCE_SIZE;
    let mu = uv.x * 2.0 - 1.0;
    let r = GROUND_R + uv.y * (TOP_R - GROUND_R);
    let pos = vec3(0.0, r, 0.0);
    let dir = vec3(sqrt(max(1.0 - mu * mu, 0.0)), mu, 0.0);
    textureStore(out_transmittance, vec2<i32>(id.xy), vec4(march_transmittance(pos, dir), 1.0));
}

// Hillaire's multi-scattering: per (sun zenith, altitude), integrate 2nd-order
// luminance L2 and transfer f over 64 sphere directions with an isotropic
// phase, then the geometric series Psi = L2 / (1 - f).
@compute @workgroup_size(8, 8, 1)
fn cs_multiscatter(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 32u || id.y >= 32u { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / 32.0;
    let mu_sun = uv.x * 2.0 - 1.0;
    let r = GROUND_R + uv.y * (TOP_R - GROUND_R);
    let pos = vec3(0.0, r, 0.0);
    let sun_dir = vec3(sqrt(max(1.0 - mu_sun * mu_sun, 0.0)), mu_sun, 0.0);

    var lum = vec3(0.0);
    var f_ms = vec3(0.0);
    let n = 8u;
    for (var a = 0u; a < n; a++) {
        for (var b = 0u; b < n; b++) {
            let theta = PI * (f32(a) + 0.5) / f32(n);
            let phi = 2.0 * PI * (f32(b) + 0.5) / f32(n);
            let dir = vec3(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi));
            let dw = sin(theta) * (PI / f32(n)) * (2.0 * PI / f32(n));

            var t_max = ray_sphere(pos, dir, TOP_R);
            let t_ground = ray_sphere(pos, dir, GROUND_R);
            if t_ground > 0.0 { t_max = t_ground; }
            if t_max <= 0.0 { continue; }
            let dt = t_max / 20.0;
            var throughput = vec3(1.0);
            for (var s = 0u; s < 20u; s++) {
                let p = pos + dir * ((f32(s) + 0.5) * dt);
                let pr = length(p);
                let m = media_at(pr - GROUND_R);
                let scatter = m.rayleigh + vec3(m.mie);
                let sun_t = sample_transmittance(pr, dot(p / pr, sun_dir));
                let step_t = exp(-m.extinction * dt);
                let inv_ext = 1.0 / max(m.extinction, vec3(1e-6));
                // Energy-conserving in-step integration (Hillaire eq. 6).
                lum += throughput * (scatter * sun_t - scatter * sun_t * step_t) * inv_ext
                    * (1.0 / (4.0 * PI)) * dw;
                f_ms += throughput * (scatter - scatter * step_t) * inv_ext
                    * (1.0 / (4.0 * PI)) * dw;
                throughput *= step_t;
            }
        }
    }
    let psi = lum / max(vec3(1.0) - f_ms, vec3(1e-4));
    textureStore(out_multiscatter, vec2<i32>(id.xy), vec4(psi, 1.0));
}

// Sky-view LUT addressing: u = world azimuth / 2pi; v = elevation with a
// square warp that concentrates texels at the horizon. sky.wgsl inverts this
// mapping — change BOTH or the sky tears.
fn skyview_elevation(v: f32) -> f32 {
    let c = v * 2.0 - 1.0;
    return sign(c) * c * c * 0.5 * PI;
}

fn march_scattering(pos: vec3<f32>, dir: vec3<f32>, sun_dir: vec3<f32>, steps: u32, t_cap: f32) -> vec4<f32> {
    var t_max = ray_sphere(pos, dir, TOP_R);
    let t_ground = ray_sphere(pos, dir, GROUND_R);
    if t_ground > 0.0 { t_max = t_ground; }
    if t_cap > 0.0 { t_max = min(t_max, t_cap); }
    if t_max <= 0.0 { return vec4(0.0, 0.0, 0.0, 1.0); }
    let cos_sun = dot(dir, sun_dir);
    let p_rayl = rayleigh_phase(cos_sun);
    let p_mie = mie_phase(cos_sun);
    let dt = t_max / f32(steps);
    var lum = vec3(0.0);
    var throughput = vec3(1.0);
    for (var i = 0u; i < steps; i++) {
        let p = pos + dir * ((f32(i) + 0.5) * dt);
        let pr = length(p);
        let m = media_at(pr - GROUND_R);
        let mu_sun = dot(p / pr, sun_dir);
        let s = (m.rayleigh * p_rayl + vec3(m.mie * p_mie)) * sample_transmittance(pr, mu_sun)
            + (m.rayleigh + vec3(m.mie)) * sample_multiscatter(pr, mu_sun);
        let step_t = exp(-m.extinction * dt);
        lum += throughput * (s - s * step_t) / max(m.extinction, vec3(1e-6));
        throughput *= step_t;
    }
    return vec4(lum, dot(throughput, vec3(1.0 / 3.0)));
}

@compute @workgroup_size(8, 8, 1)
fn cs_skyview(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 192u || id.y >= 108u { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / SKYVIEW_SIZE;
    let azimuth = (uv.x * 2.0 - 1.0) * PI;
    let elev = skyview_elevation(uv.y);
    let dir = vec3(cos(elev) * sin(azimuth), sin(elev), -cos(elev) * cos(azimuth));
    let pos = vec3(0.0, GROUND_R + max(atm.camera.w, 5e-4), 0.0);
    let result = march_scattering(pos, dir, atm.sun.xyz, 32u, -1.0);
    textureStore(out_skyview, vec2<i32>(id.xy), vec4(result.rgb * atm.sun_radiance.rgb, 1.0));
}
```

- [ ] **Step 3: Implement `SkyLuts` in `atmosphere.rs`**

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AtmUniform {
    pub inv_view_proj: [[f32; 4]; 4],
    /// xyz world camera pos (meters), w altitude in km.
    pub camera: [f32; 4],
    /// xyz toward the sun (world space).
    pub sun: [f32; 4],
    /// rgb top-of-atmosphere sun radiance.
    pub sun_radiance: [f32; 4],
}

const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

fn lut_texture(device: &wgpu::Device, label: &str, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LUT_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn sampled_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn storage_entry(binding: u32, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: LUT_FORMAT,
            view_dimension: dim,
        },
        count: None,
    }
}

/// One (group0, group1, pipeline) triple per LUT entry point.
struct LutPass {
    pipeline: wgpu::ComputePipeline,
    in_group: wgpu::BindGroup,
    out_group: wgpu::BindGroup,
    workgroups: (u32, u32, u32),
}

pub struct SkyLuts {
    uniform: wgpu::Buffer,
    pub skyview_view: wgpu::TextureView,
    transmittance_view: wgpu::TextureView,
    multiscatter_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    passes: Vec<LutPass>, // [transmittance, multiscatter, skyview]
    statics_done: bool,
}

impl SkyLuts {
    pub fn new(device: &wgpu::Device, shader_source: &str) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atm uniform"),
            size: std::mem::size_of::<AtmUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let transmittance_view = lut_texture(device, "transmittance lut", 256, 64);
        let multiscatter_view = lut_texture(device, "multiscatter lut", 32, 32);
        let skyview_view = lut_texture(device, "skyview lut", 192, 108);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lut"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let mut luts = Self {
            uniform,
            skyview_view,
            transmittance_view,
            multiscatter_view,
            sampler,
            passes: Vec::new(),
            statics_done: false,
        };
        luts.build_passes(device, shader_source);
        luts
    }

    fn build_passes(&mut self, device: &wgpu::Device, shader_source: &str) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky luts"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        // (entry, group0 entries, group0 resources, out binding, out view, workgroups)
        let uni = wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() };
        let tr = |b| wgpu::BindGroupEntry {
            binding: b,
            resource: wgpu::BindingResource::TextureView(&self.transmittance_view),
        };
        let ms = wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::TextureView(&self.multiscatter_view),
        };
        let smp = wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::Sampler(&self.sampler),
        };

        let specs: [(&str, Vec<wgpu::BindGroupLayoutEntry>, Vec<wgpu::BindGroupEntry<'_>>, u32, &wgpu::TextureView, (u32, u32, u32)); 3] = [
            (
                "cs_transmittance",
                vec![uniform_entry(0)],
                vec![uni.clone()],
                0,
                &self.transmittance_view,
                (256 / 8, 64 / 8, 1),
            ),
            (
                "cs_multiscatter",
                vec![uniform_entry(0), sampled_entry(1), sampler_entry(3)],
                vec![uni.clone(), tr(1), smp.clone()],
                1,
                &self.multiscatter_view,
                (32 / 8, 32 / 8, 1),
            ),
            (
                "cs_skyview",
                vec![uniform_entry(0), sampled_entry(1), sampled_entry(2), sampler_entry(3)],
                vec![uni.clone(), tr(1), ms.clone(), smp.clone()],
                2,
                &self.skyview_view,
                (192 / 8, 108_u32.div_ceil(8), 1),
            ),
        ];

        self.passes = specs
            .into_iter()
            .map(|(entry, in_entries, in_resources, out_binding, out_view, workgroups)| {
                let in_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(entry),
                    entries: &in_entries,
                });
                let out_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(entry),
                    entries: &[storage_entry(out_binding, wgpu::TextureViewDimension::D2)],
                });
                let in_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(entry),
                    layout: &in_layout,
                    entries: &in_resources,
                });
                let out_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(entry),
                    layout: &out_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: out_binding,
                        resource: wgpu::BindingResource::TextureView(out_view),
                    }],
                });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(entry),
                    bind_group_layouts: &[Some(&in_layout), Some(&out_layout)],
                    immediate_size: 0,
                });
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    cache: None,
                });
                LutPass { pipeline, in_group, out_group, workgroups }
            })
            .collect();
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.build_passes(device, shader_source);
        self.statics_done = false; // recompute static LUTs with the new code
    }

    pub fn prepare(&self, queue: &wgpu::Queue, u: &AtmUniform) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(u));
    }

    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sky luts"),
            timestamp_writes,
        });
        let from = if self.statics_done { 2 } else { 0 };
        for pass in &self.passes[from..] {
            cpass.set_pipeline(&pass.pipeline);
            cpass.set_bind_group(0, &pass.in_group, &[]);
            cpass.set_bind_group(1, &pass.out_group, &[]);
            cpass.dispatch_workgroups(pass.workgroups.0, pass.workgroups.1, pass.workgroups.2);
        }
        self.statics_done = true;
    }
}
```

(If `wgpu::BindGroupEntry` is not `Clone` in wgpu 29, construct the entry vectors inline per spec instead of cloning — same content.)

- [ ] **Step 4: Wire `app.rs`**

1. Field `sky_luts: Option<crate::render::atmosphere::SkyLuts>`; pass labels gain a leading `luts` slot:

```rust
const PASS_LABELS: &[&str] = &["luts", "shadow0", "shadow1", "shadow2", "main", "post"];
const PASS_LUTS: usize = 0;
const PASS_SHADOW0: usize = 1;
const PASS_MAIN: usize = 4;
const PASS_POST: usize = 5;
```

2. `resumed()`: `shaders.watch("sky_luts", shader_path("sky_luts.wgsl"));` and

```rust
let luts_src = std::fs::read_to_string(shader_path("sky_luts.wgsl")).expect("sky_luts.wgsl missing");
self.sky_luts = Some(crate::render::atmosphere::SkyLuts::new(&gpu.device, &luts_src));
```

3. Hot-reload arm: `"sky_luts" => if let Some(l) = self.sky_luts.as_mut() { l.swap_shader(&gpu.device, &source); }`.

4. In `render()` next to the other `prepare` calls (the `inv_view_proj`/`view_proj` already exist there):

```rust
if let Some(luts) = self.sky_luts.as_ref() {
    luts.prepare(&gpu.queue, &crate::render::atmosphere::AtmUniform {
        inv_view_proj: view_proj.inverse().to_cols_array_2d(),
        camera: [self.camera.position.x, self.camera.position.y, self.camera.position.z, altitude_km],
        sun: [self.day.sun_dir().x, self.day.sun_dir().y, self.day.sun_dir().z, 0.0],
        sun_radiance: [
            crate::render::atmosphere::SUN_RADIANCE.x,
            crate::render::atmosphere::SUN_RADIANCE.y,
            crate::render::atmosphere::SUN_RADIANCE.z,
            0.0,
        ],
    });
}
```

(The LUTs always track the *sun* — at night the sky shows proper twilight/dark from the sun below the horizon; the moon is a lighting-only construct.)

5. First thing inside the encoder (before the shadow passes):

```rust
let luts_writes = self.timer.as_ref().and_then(|t| t.compute_writes(PASS_LUTS));
if let Some(luts) = self.sky_luts.as_mut() {
    luts.encode(&mut encoder, luts_writes);
}
```

- [ ] **Step 5: Run tests, clippy, smoke-run**

```bash
cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 20 && kill $APP_PID
```
Expected: no visual change yet (nothing samples the LUTs); HUD shows a `luts` entry — large on the first timed frame (static LUTs), then well under 0.1 ms.

- [ ] **Step 6: Commit**

```bash
git add git-craft/assets/shaders/sky_luts.wgsl git-craft/src/render/atmosphere.rs git-craft/src/app.rs
git commit -m "feat: compute Hillaire transmittance, multi-scatter, and sky-view LUTs (m5 rung 2)"
```

---

### Task 8: Sky background from the sky-view LUT (rung 2)

**Files:**
- Create: `git-craft/assets/shaders/sky.wgsl`
- Modify: `git-craft/src/render/atmosphere.rs` (add `SkyPass`), `git-craft/src/render/terrain.rs` (one accessor), `git-craft/src/app.rs`

The flat `DayCycle::sky_color()` clear is replaced by a real sky: a fullscreen triangle at depth 1.0 drawn inside the main pass after terrain (compare LessEqual, no depth write — HSR kills every covered pixel), sampling the sky-view LUT plus a sun disc. The main pass clear becomes black. `DayCycle::sky_color()` survives as the terrain *ambient* term only.

- [ ] **Step 1: Create `assets/shaders/sky.wgsl`**

```wgsl
// Sky background: fullscreen triangle at depth 1.0 (LessEqual, no write),
// drawn after opaque terrain. Samples the sky-view LUT and adds a sun disc.
// The FrameUniform struct must match terrain.wgsl / render/terrain.rs.

struct FrameUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,
    sky: vec4<f32>,
    sun: vec4<f32>,       // xyz light dir, w 1=sun 0=moon
    sun_color: vec4<f32>, // rgb light radiance
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniform;
@group(1) @binding(0) var skyview: texture_2d<f32>;
@group(1) @binding(1) var sky_samp: sampler;

const PI: f32 = 3.14159265;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0); // z = far plane
    out.uv = uv;
    return out;
}

// Inverse of sky_luts.wgsl's mapping (azimuth linear, elevation square warp).
fn skyview_uv(dir: vec3<f32>) -> vec2<f32> {
    let azimuth = atan2(dir.x, -dir.z);
    let elev = asin(clamp(dir.y, -1.0, 1.0));
    let c = sign(elev) * sqrt(abs(elev) / (0.5 * PI));
    return vec2(azimuth / (2.0 * PI) + 0.5, c * 0.5 + 0.5);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let ndc = vec4(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, 1.0, 1.0);
    let world = frame.inv_view_proj * ndc;
    let dir = normalize(world.xyz / world.w - frame.camera.xyz);
    var color = textureSampleLevel(skyview, sky_samp, skyview_uv(dir), 0.0).rgb;

    // Sun disc (real sun only — frame.sun is the moon at night), angular
    // radius ~0.27 deg with a soft limb; sun_color already carries the
    // atmospheric transmittance, so the disc reddens at sunset for free.
    if frame.sun.w > 0.5 && dir.y > -0.1 {
        let d = dot(dir, frame.sun.xyz);
        let disc = smoothstep(cos(0.0055), cos(0.0035), d);
        color += frame.sun_color.rgb * 40.0 * disc;
    }
    return vec4(color, 1.0);
}
```

- [ ] **Step 2: Add `SkyPass` to `atmosphere.rs`**

The frame uniform lives in `TerrainRenderer`; reuse its layout and bind group instead of duplicating the buffer. Add to `terrain.rs`:

```rust
    pub fn camera_layout(&self) -> &wgpu::BindGroupLayout {
        &self.camera_layout
    }

    pub fn camera_bind_group(&self) -> &wgpu::BindGroup {
        &self.camera_bind_group
    }
```

In `atmosphere.rs`:

```rust
pub struct SkyPass {
    pipeline: wgpu::RenderPipeline,
    lut_layout: wgpu::BindGroupLayout,
    lut_bind_group: wgpu::BindGroup,
}

impl SkyPass {
    pub fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        skyview: &wgpu::TextureView,
        shader_source: &str,
    ) -> Self {
        let lut_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // Repeat on U: the azimuth seam at u = 0/1 must filter across.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("skyview"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let lut_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky"),
            layout: &lut_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(skyview) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let pipeline = Self::build_pipeline(device, camera_layout, &lut_layout, shader_source);
        Self { pipeline, lut_layout, lut_bind_group }
    }

    fn build_pipeline(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        lut_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(camera_layout), Some(lut_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::render::depth::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::render::targets::HDR_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn swap_shader(
        &mut self,
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) {
        self.pipeline = Self::build_pipeline(device, camera_layout, &self.lut_layout, shader_source);
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass<'_>, camera_bind_group: &wgpu::BindGroup) {
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, camera_bind_group, &[]);
        rpass.set_bind_group(1, &self.lut_bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
}
```

- [ ] **Step 3: Wire `app.rs`**

1. Field `sky_pass: Option<crate::render::atmosphere::SkyPass>`; `shaders.watch("sky", shader_path("sky.wgsl"));` and in `resumed()` (after terrain and sky_luts):

```rust
let sky_src = std::fs::read_to_string(shader_path("sky.wgsl")).expect("sky.wgsl missing");
let terrain_ref = self.terrain.as_ref().unwrap();
let luts_ref = self.sky_luts.as_ref().unwrap();
self.sky_pass = Some(crate::render::atmosphere::SkyPass::new(
    &gpu.device,
    terrain_ref.camera_layout(),
    &luts_ref.skyview_view,
    &sky_src,
));
```

2. Hot-reload arm:

```rust
"sky" => {
    if let (Some(s), Some(t)) = (self.sky_pass.as_mut(), self.terrain.as_ref()) {
        s.swap_shader(&gpu.device, t.camera_layout(), &source);
    }
}
```

3. Main pass: clear color becomes black (`wgpu::Color::BLACK` — delete the `sky` capture used for the clear), and after `terrain.draw(&mut rpass)`, before the outline:

```rust
if let (Some(sky), Some(terrain)) = (self.sky_pass.as_ref(), self.terrain.as_ref()) {
    sky.draw(&mut rpass, terrain.camera_bind_group());
}
```

- [ ] **Step 4: Run tests, clippy, smoke-run**

```bash
cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 30 && kill $APP_PID
```
Expected: blue gradient sky with a bright horizon band; visible sun disc that travels; at sunset (`T` is not bound — wait through a cycle or temporarily set `DayCycle::new` time near 0.45) the horizon goes orange→red with proper twilight after sundown; night sky is near-black, no seam looking north (azimuth wrap), no tearing at the horizon when pitching.

- [ ] **Step 5: Commit**

```bash
git add git-craft/assets/shaders/sky.wgsl git-craft/src/render/atmosphere.rs \
        git-craft/src/render/terrain.rs git-craft/src/app.rs
git commit -m "feat: draw the sky from the sky-view LUT with a sun disc (m5 rung 2)"
```

---

### Task 9: Aerial-perspective fog on terrain (completes rung 2)

**Files:**
- Modify: `git-craft/assets/shaders/sky_luts.wgsl` (aerial froxel entry), `git-craft/src/render/atmosphere.rs` (4th pass + 3D LUT), `git-craft/src/render/terrain.rs` (bind group 3), `git-craft/assets/shaders/terrain.wgsl` (apply fog), `git-craft/src/app.rs`

A 32×32×32 froxel LUT: each XY texel is a screen direction, each Z slice an exaggerated view distance; the cell stores accumulated in-scatter (rgb) and mean transmittance (a). Terrain composites `color * ap.a + ap.rgb`. Real aerial perspective over 384 m is invisible — the spec wants shader-pack fog, so distance is scaled by an artistic constant.

- [ ] **Step 1: Add the aerial entry to `sky_luts.wgsl`**

New binding and constants next to the existing ones:

```wgsl
@group(1) @binding(3) var out_aerial: texture_storage_3d<rgba16float, write>;

const AP_SIZE: f32 = 32.0;
// Far froxel slice distance in km. terrain.wgsl divides by the same value.
const AP_MAX_KM: f32 = 10.0;
```

New entry point at the bottom:

```wgsl
// Aerial-perspective froxels: in-scatter and transmittance from the camera
// to 32 exaggerated view distances per screen cell.
@compute @workgroup_size(4, 4, 4)
fn cs_aerial(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 32u || id.y >= 32u || id.z >= 32u { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / AP_SIZE;
    let ndc = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.5, 1.0);
    let world = atm.inv_view_proj * ndc;
    let dir = normalize(world.xyz / world.w - atm.camera.xyz);
    // Atmosphere is horizontally homogeneous: only altitude matters.
    let pos = vec3(0.0, GROUND_R + max(atm.camera.w, 5e-4), 0.0);
    let t_end = (f32(id.z) + 1.0) / AP_SIZE * AP_MAX_KM;
    let result = march_scattering(pos, dir, atm.sun.xyz, 16u, t_end);
    textureStore(out_aerial, vec3<i32>(id), vec4(result.rgb * atm.sun_radiance.rgb, result.a));
}
```

(`march_scattering` already takes the `t_cap` parameter — Task 7 defined it that way for exactly this reuse.)

- [ ] **Step 2: Extend `SkyLuts`**

In `atmosphere.rs`:

1. Public scale constant the app feeds into `FrameParams`' `params.z` slot:

```rust
/// Artistic fog-distance scale: world meters × this = atmosphere km marched.
/// Physically 0.001; 0.02 marches 20× the air so the 384-block render
/// distance picks up shader-pack-grade aerial perspective. Tune live by
/// editing AP_MAX_KM / this pair (hot-reload + restart respectively).
pub const AP_KM_PER_METER: f32 = 0.02;
```

2. A 3D LUT texture + view (`aerial_view`), created in `new` (usage `STORAGE_BINDING | TEXTURE_BINDING`, dimension `D3`, size 32×32×32, same `LUT_FORMAT`):

```rust
        let aerial = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("aerial lut"),
            size: wgpu::Extent3d { width: 32, height: 32, depth_or_array_layers: 32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: LUT_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let aerial_view = aerial.create_view(&wgpu::TextureViewDescriptor::default());
```

store as `pub aerial_view: wgpu::TextureView`.

3. A 4th spec row in `build_passes` (runs every frame — `encode`'s per-frame range becomes `passes[2..]`):

```rust
            (
                "cs_aerial",
                vec![uniform_entry(0), sampled_entry(1), sampled_entry(2), sampler_entry(3)],
                vec![uni.clone(), tr(1), ms.clone(), smp.clone()],
                3,
                &self.aerial_view,
                (32 / 4, 32 / 4, 32 / 4),
            ),
```

with the storage layout for this row using `wgpu::TextureViewDimension::D3` — extend the spec tuple with the storage dimension (D2 for the first three rows, D3 here) and pass it to `storage_entry(out_binding, dim)`.

- [ ] **Step 3: Terrain bind group 3 + fog application**

`terrain.rs` — a fourth layout (binding 0: `texture_3d<f32>` float filterable FRAGMENT; binding 1: filtering sampler FRAGMENT), built like `shadow_layout` in Task 5 with `view_dimension: wgpu::TextureViewDimension::D3`; field `aerial_bind_group: Option<wgpu::BindGroup>` + own linear-clamp sampler; pipeline layout becomes `&[Some(camera_layout), Some(quads_layout), Some(shadow_layout), Some(aerial_layout)]` (the full 4 bind groups);

```rust
    pub fn attach_aerial(&mut self, device: &wgpu::Device, lut: &wgpu::TextureView) {
        self.aerial_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain aerial"),
            layout: &self.aerial_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(lut) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.aerial_sampler) },
            ],
        }));
    }
```

`draw()` guards and binds group 3 exactly like group 2. `write_frame`'s params line becomes:

```rust
            params: [
                p.viewport.0 as f32,
                p.viewport.1 as f32,
                crate::render::atmosphere::AP_KM_PER_METER,
                0.0,
            ],
```

`terrain.wgsl` — new bindings:

```wgsl
@group(3) @binding(0) var aerial_lut: texture_3d<f32>;
@group(3) @binding(1) var aerial_samp: sampler;
```

and the end of `fs_main` becomes:

```wgsl
    let lit = in.albedo * (direct + ambient + torch);
    // Aerial perspective: froxel slice indexed by exaggerated view distance.
    // 10.0 = AP_MAX_KM in sky_luts.wgsl.
    let screen_uv = in.clip.xy / frame.params.xy;
    let slice = clamp(view_dist * frame.params.z / 10.0, 0.0, 1.0);
    let ap = textureSampleLevel(aerial_lut, aerial_samp, vec3(screen_uv, slice), 0.0);
    return vec4(lit * ap.a + ap.rgb, 1.0);
```

- [ ] **Step 4: Wire `app.rs`**

In `resumed()`, next to `attach_shadow`:

```rust
if let (Some(terrain), Some(luts)) = (self.terrain.as_mut(), self.sky_luts.as_ref()) {
    terrain.attach_aerial(&gpu.device, &luts.aerial_view);
}
```

- [ ] **Step 5: Run tests, clippy, smoke-run**

```bash
cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 30 && kill $APP_PID
```
Expected: distant terrain haze-blues into the sky and the horizon line melts into the fog instead of cutting; near geometry unchanged; fog tint follows time of day (warm at sunset). HUD `luts` still < 0.2 ms.

- [ ] **Step 6: Commit (rung 2 complete)**

```bash
git add git-craft/assets/shaders/sky_luts.wgsl git-craft/assets/shaders/terrain.wgsl \
        git-craft/src/render/atmosphere.rs git-craft/src/render/terrain.rs git-craft/src/app.rs
git commit -m "feat: apply aerial-perspective fog from a froxel LUT (m5 rung 2)"
```

---

### Task 10: Bloom down/up chain (rung 3)

**Files:**
- Create: `git-craft/src/render/bloom.rs`, `git-craft/assets/shaders/bloom.wgsl`
- Modify: `git-craft/src/render/targets.rs` (bloom mip chain), `git-craft/src/render/post.rs` + `git-craft/assets/shaders/post.wgsl` (bloom mix), `git-craft/src/render/mod.rs`, `git-craft/src/app.rs`

Jimenez 2014 bloom: 13-tap downsample (Karis average on the first level to kill fireflies) through a half-res mip chain, then 3×3 tent upsample drawn **additively** back up; post mixes mip 0 into the HDR color. No threshold — energy-based, shader-pack style. The whole chain is one timer slot (`render_writes_begin` on the first pass, `_end` on the last — Task 1 built exactly this API).

- [ ] **Step 1: Write the failing tests**

In `targets.rs` add a test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloom_mip_count_scales_with_resolution() {
        assert_eq!(bloom_mip_count(1512, 982), 6, "native half-res gets the full chain");
        assert_eq!(bloom_mip_count(20, 20), 2);
        assert_eq!(bloom_mip_count(8, 8), 1, "never zero mips");
    }
}
```

In `bloom.rs` (new file, tests at bottom):

```rust
    #[test]
    fn bloom_uniform_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<BloomUniform>(), 16);
        assert_eq!(std::mem::offset_of!(BloomUniform, karis), 8);
    }
```

Run: `cargo test --manifest-path git-craft/Cargo.toml bloom` — FAIL.

- [ ] **Step 2: Extend `targets.rs`**

```rust
/// Mip count for the half-res bloom chain: down to ~8 px, capped at 6.
pub fn bloom_mip_count(w: u32, h: u32) -> u32 {
    let (mut w, mut h, mut n) = (w, h, 1);
    while n < 6 && w >= 16 && h >= 16 {
        w /= 2;
        h /= 2;
        n += 1;
    }
    n
}
```

`RenderTargets` gains:

```rust
    pub bloom_views: Vec<wgpu::TextureView>, // one per mip
    pub bloom_sizes: Vec<(u32, u32)>,
```

built in `new` after the HDR texture:

```rust
        let half = ((width / 2).max(1), (height / 2).max(1));
        let mips = bloom_mip_count(half.0, half.1);
        let bloom = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bloom chain"),
            size: wgpu::Extent3d { width: half.0, height: half.1, depth_or_array_layers: 1 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_views = (0..mips)
            .map(|i| {
                bloom.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("bloom mip"),
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let bloom_sizes = (0..mips)
            .map(|i| ((half.0 >> i).max(1), (half.1 >> i).max(1)))
            .collect();
```

- [ ] **Step 3: Create `assets/shaders/bloom.wgsl`**

```wgsl
// Jimenez 2014 (Advanced Warfare) bloom: 13-tap downsample with optional
// Karis average on the first level; 3x3 tent upsample drawn additively.

struct BloomUniform {
    texel: vec2<f32>,   // SOURCE texel size
    karis: f32,         // 1.0 only on the HDR -> mip0 downsample
    intensity: f32,     // tent upsample scale
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> u: BloomUniform;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn s(uv: vec2<f32>, dx: f32, dy: f32) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv + vec2(dx, dy) * u.texel, 0.0).rgb;
}

fn karis_weight(c: vec3<f32>) -> f32 {
    return 1.0 / (1.0 + dot(c, vec3(0.2126, 0.7152, 0.0722)));
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let a = s(uv, -2.0, -2.0); let b = s(uv, 0.0, -2.0); let c = s(uv, 2.0, -2.0);
    let d = s(uv, -2.0, 0.0);  let e = s(uv, 0.0, 0.0);  let f = s(uv, 2.0, 0.0);
    let g = s(uv, -2.0, 2.0);  let h = s(uv, 0.0, 2.0);  let i = s(uv, 2.0, 2.0);
    let j = s(uv, -1.0, -1.0); let k = s(uv, 1.0, -1.0);
    let l = s(uv, -1.0, 1.0);  let m = s(uv, 1.0, 1.0);
    // Five overlapping 2x2 blocks: center half weight, corners eighth each.
    let b0 = (j + k + l + m) * 0.25;
    let b1 = (a + b + d + e) * 0.25;
    let b2 = (b + c + e + f) * 0.25;
    let b3 = (d + e + g + h) * 0.25;
    let b4 = (e + f + h + i) * 0.25;
    if u.karis > 0.5 {
        let w0 = karis_weight(b0); let w1 = karis_weight(b1); let w2 = karis_weight(b2);
        let w3 = karis_weight(b3); let w4 = karis_weight(b4);
        let col = b0 * w0 * 0.5 + (b1 * w1 + b2 * w2 + b3 * w3 + b4 * w4) * 0.125;
        let wsum = w0 * 0.5 + (w1 + w2 + w3 + w4) * 0.125;
        return vec4(col / max(wsum, 1e-4), 1.0);
    }
    return vec4(b0 * 0.5 + (b1 + b2 + b3 + b4) * 0.125, 1.0);
}

@fragment
fn fs_up(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    var col = s(uv, -1.0, -1.0) + s(uv, 1.0, -1.0) + s(uv, -1.0, 1.0) + s(uv, 1.0, 1.0);
    col += (s(uv, -1.0, 0.0) + s(uv, 1.0, 0.0) + s(uv, 0.0, -1.0) + s(uv, 0.0, 1.0)) * 2.0;
    col += s(uv, 0.0, 0.0) * 4.0;
    return vec4(col / 16.0 * u.intensity, 1.0);
}
```

- [ ] **Step 4: Implement `BloomPass` in `bloom.rs`**

```rust
use crate::render::targets::{RenderTargets, HDR_FORMAT};
use crate::render::timestamps::GpuTimer;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BloomUniform {
    texel: [f32; 2],
    karis: f32,
    intensity: f32,
}

/// Dynamic-offset slot per draw (universal uniform alignment).
const SLOT: u64 = 256;
/// Max draws: 6 down + 5 up.
const MAX_SLOTS: u64 = 16;

pub struct BloomPass {
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    /// Bind group per source view: [0] = HDR, [i>=1] = bloom mip i-1.
    src_groups: Vec<wgpu::BindGroup>,
    mip_count: usize,
}

impl BloomPass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &RenderTargets,
        shader_source: &str,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bloom uniforms"),
            size: SLOT * MAX_SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (down, up) = Self::build_pipelines(device, &layout, shader_source);
        let mut pass = Self {
            down,
            up,
            layout,
            sampler,
            uniform_buffer,
            src_groups: Vec::new(),
            mip_count: 0,
        };
        pass.set_targets(device, queue, targets);
        pass
    }

    fn build_pipelines(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        let make = |entry: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        (make("fs_down", wgpu::BlendState::REPLACE), make("fs_up", additive))
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        let (down, up) = Self::build_pipelines(device, &self.layout, shader_source);
        self.down = down;
        self.up = up;
    }

    /// Rebuild bind groups + per-draw uniforms after target (re)creation.
    pub fn set_targets(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, targets: &RenderTargets) {
        let mut views: Vec<&wgpu::TextureView> = vec![&targets.hdr_view];
        views.extend(targets.bloom_views.iter());
        self.src_groups = views
            .iter()
            .map(|view| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bloom src"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.uniform_buffer,
                                offset: 0,
                                size: Some(std::num::NonZeroU64::new(std::mem::size_of::<BloomUniform>() as u64).unwrap()),
                            }),
                        },
                    ],
                })
            })
            .collect();
        self.mip_count = targets.bloom_views.len();

        // Per-draw uniforms. Down pass i samples [HDR, mip0, ..]: source size
        // is full-res for i == 0, else mip i-1. Up pass j samples mip j+1.
        let full = (targets.width, targets.height);
        for i in 0..self.mip_count {
            let src = if i == 0 { full } else { targets.bloom_sizes[i - 1] };
            let u = BloomUniform {
                texel: [1.0 / src.0 as f32, 1.0 / src.1 as f32],
                karis: if i == 0 { 1.0 } else { 0.0 },
                intensity: 1.0,
            };
            queue.write_buffer(&self.uniform_buffer, i as u64 * SLOT, bytemuck::bytes_of(&u));
        }
        for j in 0..self.mip_count.saturating_sub(1) {
            let src = targets.bloom_sizes[j + 1];
            let u = BloomUniform {
                texel: [1.0 / src.0 as f32, 1.0 / src.1 as f32],
                karis: 0.0,
                intensity: 1.0,
            };
            queue.write_buffer(
                &self.uniform_buffer,
                (self.mip_count + j) as u64 * SLOT,
                bytemuck::bytes_of(&u),
            );
        }
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets,
        timer: Option<&GpuTimer>,
        pass_slot: usize,
    ) {
        let n = self.mip_count;
        let mut draw = |dst: &wgpu::TextureView,
                        src_group: usize,
                        slot: u64,
                        pipeline: &wgpu::RenderPipeline,
                        load: wgpu::LoadOp<wgpu::Color>,
                        ts: Option<wgpu::RenderPassTimestampWrites<'_>>,
                        encoder: &mut wgpu::CommandEncoder| {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: ts,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, &self.src_groups[src_group], &[(slot * SLOT) as u32]);
            rpass.draw(0..3, 0..1);
        };
        for i in 0..n {
            let ts = if i == 0 { timer.and_then(|t| t.render_writes_begin(pass_slot)) } else { None };
            draw(
                &targets.bloom_views[i],
                i, // [0]=HDR, [i]=mip i-1
                i as u64,
                &self.down,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                ts,
                encoder,
            );
        }
        for j in (0..n.saturating_sub(1)).rev() {
            let ts = if j == 0 { timer.and_then(|t| t.render_writes_end(pass_slot)) } else { None };
            draw(
                &targets.bloom_views[j],
                j + 2, // mip j+1
                (n + j) as u64,
                &self.up,
                wgpu::LoadOp::Load, // additive onto the downsampled content
                ts,
                encoder,
            );
        }
    }
}
```

(plus the layout test from Step 1 at the bottom; add `pub mod bloom;` to `render/mod.rs`).

- [ ] **Step 5: Mix bloom in the post pass**

`post.wgsl`: add `@group(0) @binding(2) var bloom_tex: texture_2d<f32>;`, `const BLOOM_STRENGTH: f32 = 0.06;`, and the fragment becomes:

```wgsl
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let hdr = textureSampleLevel(hdr_tex, hdr_samp, in.uv, 0.0).rgb;
    let bloom = textureSampleLevel(bloom_tex, hdr_samp, in.uv, 0.0).rgb;
    let color = mix(hdr, bloom, BLOOM_STRENGTH);
    return vec4(color, 1.0);
}
```

`post.rs`: layout gains a binding-2 texture entry (copy of binding 0's entry with `binding: 2`); `new` and `set_input` take `bloom_view: &wgpu::TextureView` (bind `targets.bloom_views[0]`) and add it to the bind group.

- [ ] **Step 6: Wire `app.rs`**

Field `bloom: Option<crate::render::bloom::BloomPass>`; `shaders.watch("bloom", shader_path("bloom.wgsl"));`; build in `resumed()` after targets (`BloomPass::new(&gpu.device, &gpu.queue, &targets, &bloom_src)`); hot-reload arm calls `swap_shader`; labels:

```rust
const PASS_LABELS: &[&str] = &["luts", "shadow0", "shadow1", "shadow2", "main", "bloom", "post"];
const PASS_BLOOM: usize = 5;
const PASS_POST: usize = 6;
```

Encode after the main pass, before post:

```rust
if let (Some(bloom), Some(targets)) = (self.bloom.as_ref(), self.targets.as_ref()) {
    bloom.encode(&mut encoder, targets, self.timer.as_ref(), PASS_BLOOM);
}
```

Resize: `bloom.set_targets(&gpu.device, &gpu.queue, targets)` next to `post.set_input(...)` (post now also rebinds `&targets.bloom_views[0]`).

- [ ] **Step 7: Run tests, clippy, smoke-run**

```bash
cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 30 && kill $APP_PID
```
Expected: sun disc and sunset horizon glow softly; torch-lit areas at night bleed warm light; no flicker from fireflies; image not washed out (strength 0.06 is subtle). HUD `bloom` ≈ 0.4 ms (spec budget).

- [ ] **Step 8: Commit**

```bash
git add git-craft/src/render/bloom.rs git-craft/assets/shaders/bloom.wgsl git-craft/src/render/targets.rs \
        git-craft/src/render/post.rs git-craft/assets/shaders/post.wgsl git-craft/src/render/mod.rs git-craft/src/app.rs
git commit -m "feat: add a 13-tap bloom downsample/upsample chain (m5 rung 3)"
```

---

### Task 11: Histogram auto-exposure (rung 3)

**Files:**
- Create: `git-craft/src/render/exposure.rs`, `git-craft/assets/shaders/exposure.wgsl`
- Modify: `git-craft/src/render/post.rs` + `git-craft/assets/shaders/post.wgsl`, `git-craft/src/render/mod.rs`, `git-craft/src/app.rs`

Spec §6 pass 1's auto-exposure histogram: a 256-bin log-luminance histogram over the HDR target (stride 2), a single-workgroup resolve that computes the mean log-luminance, derives a target exposure, and adapts toward it over time. **No CPU readback** — the result lives in a 16-byte storage buffer the post pass reads directly; adaptation state stays on the GPU.

- [ ] **Step 1: Write the failing layout test**

In `exposure.rs` (new file, tests at bottom):

```rust
    #[test]
    fn exposure_uniform_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<ExposureUniform>(), 16);
        assert_eq!(std::mem::offset_of!(ExposureUniform, min_log_lum), 4);
    }
```

Run: `cargo test --manifest-path git-craft/Cargo.toml exposure::` — FAIL.

- [ ] **Step 2: Create `assets/shaders/exposure.wgsl`**

```wgsl
// Auto-exposure: 256-bin log2-luminance histogram over the HDR target
// (stride 2), then a one-workgroup resolve that adapts a smoothed exposure.
// result[0] = exposure factor, result[1] = mean log2 luminance (debug).

struct ExposureUniform {
    dt: f32,
    min_log_lum: f32,   // -12.0
    inv_log_range: f32, // 1.0 / 18.0
    log_range: f32,     // 18.0
};

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> bins: array<atomic<u32>, 256>;
@group(0) @binding(2) var<storage, read_write> result: array<f32, 4>;
@group(0) @binding(3) var<uniform> u: ExposureUniform;

var<workgroup> local_bins: array<atomic<u32>, 256>;

fn bin_of(c: vec3<f32>) -> u32 {
    let lum = dot(c, vec3(0.2126, 0.7152, 0.0722));
    if lum < 1e-4 {
        return 0u; // black bin, excluded from the mean
    }
    let l = clamp((log2(lum) - u.min_log_lum) * u.inv_log_range, 0.0, 1.0);
    return u32(l * 254.0 + 1.0);
}

@compute @workgroup_size(16, 16, 1)
fn cs_histogram(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    atomicStore(&local_bins[li], 0u);
    workgroupBarrier();
    let dim = textureDimensions(hdr_tex);
    let px = id.xy * 2u;
    if px.x < dim.x && px.y < dim.y {
        let c = textureLoad(hdr_tex, vec2<i32>(px), 0).rgb;
        atomicAdd(&local_bins[bin_of(c)], 1u);
    }
    workgroupBarrier();
    atomicAdd(&bins[li], atomicLoad(&local_bins[li]));
}

var<workgroup> w_sum: array<f32, 256>;
var<workgroup> w_count: array<f32, 256>;

@compute @workgroup_size(256, 1, 1)
fn cs_resolve(@builtin(local_invocation_index) li: u32) {
    let count = f32(atomicLoad(&bins[li]));
    atomicStore(&bins[li], 0u); // zeroed for the next frame
    w_sum[li] = count * f32(li);
    w_count[li] = select(count, 0.0, li == 0u); // skip the black bin
    workgroupBarrier();
    for (var stride = 128u; stride > 0u; stride >>= 1u) {
        if li < stride {
            w_sum[li] += w_sum[li + stride];
            w_count[li] += w_count[li + stride];
        }
        workgroupBarrier();
    }
    if li == 0u {
        let total = max(w_count[0], 1.0);
        let mean_bin = w_sum[0] / total;
        let mean_log = (mean_bin - 1.0) / 254.0 * u.log_range + u.min_log_lum;
        let avg_lum = exp2(mean_log);
        let target = clamp(0.115 / max(avg_lum, 1e-4), 0.03, 30.0);
        let prev = result[0];
        var exposure = target;
        if prev > 0.0 {
            // Eye-style adaptation: darkening adapts faster than brightening.
            let rate = select(1.2, 2.5, target < prev);
            exposure = prev + (target - prev) * (1.0 - exp(-u.dt * rate));
        }
        result[0] = exposure;
        result[1] = mean_log;
    }
}
```

- [ ] **Step 3: Implement `ExposurePass` in `exposure.rs`**

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ExposureUniform {
    dt: f32,
    min_log_lum: f32,
    inv_log_range: f32,
    log_range: f32,
}

pub struct ExposurePass {
    histogram: wgpu::ComputePipeline,
    resolve: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    bins: wgpu::Buffer,
    result: wgpu::Buffer,
    uniform: wgpu::Buffer,
}

impl ExposurePass {
    pub fn new(device: &wgpu::Device, hdr_view: &wgpu::TextureView, shader_source: &str) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("exposure"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bins = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure bins"),
            size: 256 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false, // wgpu zero-initializes
        });
        let result = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure result"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure uniform"),
            size: std::mem::size_of::<ExposureUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("exposure"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("exposure"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let histogram = make("cs_histogram");
        let resolve = make("cs_resolve");
        let bind_group = Self::build_bind_group(device, &layout, hdr_view, &bins, &result, &uniform);
        Self { histogram, resolve, layout, bind_group, bins, result, uniform }
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        bins: &wgpu::Buffer,
        result: &wgpu::Buffer,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exposure"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(hdr_view) },
                wgpu::BindGroupEntry { binding: 1, resource: bins.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: result.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
            ],
        })
    }

    pub fn set_input(&mut self, device: &wgpu::Device, hdr_view: &wgpu::TextureView) {
        self.bind_group =
            Self::build_bind_group(device, &self.layout, hdr_view, &self.bins, &self.result, &self.uniform);
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        // Pipelines only; layouts and buffers are stable.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("exposure"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("exposure"),
            bind_group_layouts: &[Some(&self.layout)],
            immediate_size: 0,
        });
        let make = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        self.histogram = make("cs_histogram");
        self.resolve = make("cs_resolve");
    }

    pub fn result_buffer(&self) -> &wgpu::Buffer {
        &self.result
    }

    pub fn prepare(&self, queue: &wgpu::Queue, dt: f32) {
        let u = ExposureUniform {
            dt,
            min_log_lum: -12.0,
            inv_log_range: 1.0 / 18.0,
            log_range: 18.0,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        width: u32,
        height: u32,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("exposure"),
            timestamp_writes,
        });
        cpass.set_pipeline(&self.histogram);
        cpass.set_bind_group(0, &self.bind_group, &[]);
        // Stride 2 in the shader: half the pixels per axis, 16x16 groups.
        cpass.dispatch_workgroups(width.div_ceil(32), height.div_ceil(32), 1);
        cpass.set_pipeline(&self.resolve);
        cpass.dispatch_workgroups(1, 1, 1);
    }
}
```

(register `pub mod exposure;`).

- [ ] **Step 4: Post pass reads the exposure**

`post.wgsl`: `@group(0) @binding(3) var<storage, read> exposure: array<f32, 4>;` and the fragment line becomes:

```wgsl
    let exposed = mix(hdr, bloom, BLOOM_STRENGTH) * max(exposure[0], 1e-3);
    return vec4(exposed, 1.0);
```

`post.rs`: layout gains binding 3 (`Storage { read_only: true }`, FRAGMENT); `new`/`set_input` take `exposure: &wgpu::Buffer` and bind it.

- [ ] **Step 5: Wire `app.rs`**

Field `exposure: Option<crate::render::exposure::ExposurePass>`; watch `"exposure"`; build in `resumed()` before `PostPass` (post needs `exposure.result_buffer()`); `exposure.prepare(&gpu.queue, dt)` next to the other prepares; labels:

```rust
const PASS_LABELS: &[&str] = &["luts", "shadow0", "shadow1", "shadow2", "main", "bloom", "exposure", "post"];
const PASS_EXPOSURE: usize = 6;
const PASS_POST: usize = 7;
```

Encode between bloom and post:

```rust
let exp_writes = self.timer.as_ref().and_then(|t| t.compute_writes(PASS_EXPOSURE));
if let Some(exposure) = self.exposure.as_ref() {
    exposure.encode(&mut encoder, gpu.config.width, gpu.config.height, exp_writes);
}
```

Resize: `exposure.set_input(&gpu.device, &targets.hdr_view)`.

- [ ] **Step 6: Run tests, clippy, smoke-run**

```bash
cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 30 && kill $APP_PID
```
Expected: walking into a cave brightens the view over ~a second; stepping back into daylight briefly over-brightens then settles (darkening adapts faster); night is moody but readable. No oscillation when looking at the sun.

- [ ] **Step 7: Commit**

```bash
git add git-craft/src/render/exposure.rs git-craft/assets/shaders/exposure.wgsl \
        git-craft/src/render/post.rs git-craft/assets/shaders/post.wgsl git-craft/src/render/mod.rs git-craft/src/app.rs
git commit -m "feat: adapt exposure from a log-luminance histogram (m5 rung 3)"
```

---

### Task 12: ACES tonemap + budget check (completes rung 3)

**Files:**
- Modify: `git-craft/assets/shaders/post.wgsl`
- Modify (if tuning needed): shader constants only

- [ ] **Step 1: Add ACES to `post.wgsl`**

Stephen Hill's fitted ACES (matrices already transposed for WGSL's column-major constructors):

```wgsl
// Stephen Hill's ACES fit: sRGB -> ACEScg-ish input transform, RRT+ODT
// rational fit, output transform. Matrices are column-major (transposed
// from the HLSL original).
const ACES_IN = mat3x3<f32>(
    vec3(0.59719, 0.07600, 0.02840),
    vec3(0.35458, 0.90834, 0.13383),
    vec3(0.04823, 0.01566, 0.83777),
);
const ACES_OUT = mat3x3<f32>(
    vec3(1.60475, -0.10208, -0.00327),
    vec3(-0.53108, 1.10813, -0.07276),
    vec3(-0.07367, -0.00605, 1.07602),
);

fn rrt_odt_fit(v: vec3<f32>) -> vec3<f32> {
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return a / b;
}

fn aces(color: vec3<f32>) -> vec3<f32> {
    return clamp(ACES_OUT * rrt_odt_fit(ACES_IN * color), vec3(0.0), vec3(1.0));
}
```

and the fragment's return becomes:

```wgsl
    return vec4(aces(exposed), 1.0);
```

(output stays linear — the sRGB swapchain encodes; do NOT add a manual gamma `pow`.)

- [ ] **Step 2: Validate, gates**

```bash
cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
```
Expected: PASS (the WGSL glob test re-validates post.wgsl).

- [ ] **Step 3: Visual + budget check**

```bash
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 60 && kill $APP_PID
```
Checks while it runs (F3/H HUD):
- Highlights roll off smoothly (sun disc has a hot core, not a clipped circle); blacks have contrast; midday no longer looks washed-out linear.
- Per-pass GPU budget vs spec §6 estimates at native res: shadows ≤ ~1.2, main ≤ ~3.0, bloom ≤ ~0.4, luts+exposure+post ≤ ~0.4 ms. Sum comfortably under 8.3 ms. If a pass blows its budget, note it in the commit body — M5b's render-scale knob is the planned valve; do not optimize blind here.
- If the overall image is too dark/bright, tune `0.115` (exposure key, exposure.wgsl) and `BLOOM_STRENGTH` (post.wgsl) via hot-reload, then re-run gates.

- [ ] **Step 4: Commit (rung 3 complete)**

```bash
git add git-craft/assets/shaders/post.wgsl git-craft/assets/shaders/exposure.wgsl
git commit -m "feat: tonemap with ACES, completing the M5a post chain (m5 rung 3)"
```

---

### Task 13: Final review and playtest hand-off

**Files:** none (verification only)

- [ ] **Step 1: Full gates on a clean tree**

```bash
git status --short   # must be empty
cargo test --manifest-path git-craft/Cargo.toml
cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
```

- [ ] **Step 2: Long smoke-run**

```bash
cargo run --release --manifest-path git-craft/Cargo.toml & APP_PID=$!
sleep 90 && kill $APP_PID
```
Watch the log for warnings (arena full, map failed, shader errors) — there must be none.

- [ ] **Step 3: Cross-cutting review**

Per the subagent workflow, a final reviewer checks the whole diff (`git diff main...HEAD`) against this plan and spec §6: bind-group budget (terrain exactly 4), TBDR ops (every attachment Clear-on-load; Discard wherever not sampled later: main-pass depth Discard, shadow Store, HDR Store, bloom mips Store), FrameUniform/ShadowUniform/AtmUniform layout tests present, no `unwrap` on GPU error paths, all 8 shaders in the glob test.

- [ ] **Step 4: User playtest checklist (report, don't gate)**

Hand the user this list:
1. Full day cycle sweep (sunrise → noon → sunset → night): sky gradient, red sunset, twilight, sun disc travel, moonlight direction flip.
2. Shadows: tree/mountain shadows move with the sun; soft edges; no acne/peter-panning; no shimmer while walking; cascade boundaries invisible.
3. Caves: pitch dark at noon beyond daylight reach; torch placement glows warm with bloom; exposure adapts entering/leaving.
4. Fog: distant terrain fades atmospherically; horizon melts into sky.
5. HUD: all 8 pass timings live; total GPU ms under 8.3 at native res; FPS vs M4 baseline.
6. Hot-reload: edit `BLOOM_STRENGTH` in post.wgsl while running — takes effect in <1 s, bad edit keeps the old pipeline with an error in the log.
7. Resize + fullscreen: no crash, no stretching.

**Do not merge to main yet** — M5b (GTAO + TAA, volumetrics, water SSR) continues on `feat/m5-shaders`; M5 merges as one milestone.

---

## Self-review

**Spec coverage (M5a scope):** frame-graph passes 1 (sky LUTs + exposure histogram — Tasks 7, 11), 2 (CSM ×3 2048², far cascades every 2–4 frames — Tasks 3–4), 3's lighting model with skylight guard (Task 5), 6's bloom + ACES + auto-exposure (Tasks 10–12); §7 moon light (Task 6); §8 per-pass HUD (Task 1); §9 shader compile tests (Task 2 glob). Deliberately deferred to M5b: GTAO+TAA+render-scale, froxel volumetrics, SSR/refraction/transparent water, normals attachment (spec pass 3's RGB10A2), depth Store, particles, soft transparents.

**Known seams an implementer must respect:**
- `PASS_LABELS` indices are renumbered in Tasks 4, 7, 10, 11 — always update ALL `PASS_*` consts in the same edit.
- terrain.wgsl's `FrameUniform`/`ShadowUniform` WGSL structs must stay byte-compatible with the Rust layout tests; sky.wgsl duplicates `FrameUniform` and sky_luts.wgsl mirrors `Atmosphere` constants — three manual sync points, all flagged with comments.
- `skyview_uv` (sky.wgsl) must invert `skyview_elevation` (sky_luts.wgsl); `AP_MAX_KM` appears in sky_luts.wgsl and as the literal `10.0` in terrain.wgsl's fog application.
- `wgpu::BindGroupEntry` cloning in `SkyLuts::build_passes` may need inlining if the type isn't `Clone` in wgpu 29 — noted in the task.
- Sky-view LUT tracks the **sun** even at night (correct twilight); the moon exists only in the lighting uniform.
