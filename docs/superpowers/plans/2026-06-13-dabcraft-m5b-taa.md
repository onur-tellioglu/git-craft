---
title: git-craft M5b — TAA (Temporal Anti-Aliasing)
date: 2026-06-13
domain: world-layer
type: enhancement
priority: high
breaking: false
db-migration: false
rls-affecting: false
slice: 5
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files:
  - git-craft/src/render/taa.rs
  - git-craft/src/render/targets.rs
  - git-craft/src/render/depth.rs
  - git-craft/src/render/post.rs
  - git-craft/src/render/bloom.rs
  - git-craft/src/render/exposure.rs
  - git-craft/src/render/mod.rs
  - git-craft/src/app.rs
  - git-craft/assets/shaders/taa.wgsl
trigger-tasks-touched: []
shared-modules-touched: []
---

# git-craft M5b — TAA Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Temporal anti-aliasing — kill the per-frame edge/shadow shimmer the M5a forward renderer shows under motion by jittering the projection sub-pixel each frame and accumulating frames into a reprojected, neighborhood-clamped history buffer.

**Architecture:** The scene is static (only the camera moves), so motion is pure camera reprojection — no per-object velocity buffer needed. Each frame the main pass renders with a sub-pixel-jittered projection into the HDR target and now **stores depth** (was discarded). A new TAA resolve pass reconstructs each pixel's world position from depth + the jittered inverse-view-proj, reprojects it through the previous frame's **unjittered** view-proj to find its history texel, clamps that history sample into the current 3×3 neighborhood colour box (kills ghosting), and blends current⊕history (~10% current). The resolved image feeds bloom → exposure → post (which previously read the raw HDR target). History is a two-texture ping-pong. New frame graph: `luts → shadows → main(jittered, depth Store) → TAA resolve → bloom → exposure → post → egui`.

**Tech Stack:** Rust (edition 2024), wgpu 29, glam 0.33, bytemuck 1.25.

**Spec:** design spec §6 ("**TAA** (not MSAA)… GTAO and volumetrics already require temporal accumulation and jitter") and §11 (render-scale knob — render-scale upsampling is **deferred** to a later M5b task; this plan delivers full-res TAA only).

**No git remote** — commit locally on the existing `feat/m5-shaders` branch (M5a is unmerged; TAA continues the same branch). Skip push/PR/issue.

**Environment:** every shell needs `export PATH="$HOME/.cargo/bin:$PATH"`. Run cargo from repo root with `--manifest-path git-craft/Cargo.toml`. Gates per task: `cargo test` + `cargo clippy --all-targets -- -D warnings`. `cargo fmt` is NOT a gate. macOS has no `timeout`; smoke-test with background-run + kill. Commit `type: what and why` (m5 suffix), English, no co-author trailers.

---

## Context primer (read before Task 1)

- `git-craft/src/app.rs` `render()`: computes `let view_proj = self.camera.view_proj(aspect);` once, uses it for the frustum, terrain `write_frame` (FrameParams), shadow `prepare`, sky LUTs, and outline. The encoder runs: sky_luts.encode (compute) → shadow.encode → main render pass (terrain.draw + sky.draw + outline.draw into the HDR target `targets.hdr_view`, depth attachment `depth_view_ref` with `StoreOp::Discard`) → bloom.encode → exposure.encode → post.draw (to swapchain) → egui. Pass labels `["luts","shadow0","shadow1","shadow2","main","bloom","exposure","post"]`.
- `git-craft/src/render/targets.rs` `RenderTargets { hdr_view, bloom_views, bloom_sizes, width, height }`, recreated on resize. `HDR_FORMAT = Rgba16Float`.
- `git-craft/src/render/depth.rs` `create_depth_view(device, w, h)` builds a `Depth32Float` texture with usage `RENDER_ATTACHMENT` only and returns just the view. TAA must SAMPLE depth, so it needs `TEXTURE_BINDING` too and the pass must `Store`.
- `git-craft/src/render/post.rs` `PostPass::new(device, surface_format, hdr_view, bloom_view, exposure, src)` and `set_input(device, hdr_view, bloom_view, exposure)`; samples HDR at binding 0. After TAA, post's HDR input becomes the **resolved** texture, not `targets.hdr_view`.
- `git-craft/src/render/bloom.rs` `BloomPass::set_targets(device, queue, &targets)` builds its source bind groups with `[0]=targets.hdr_view, [i>=1]=bloom mip i-1`. After TAA, bloom's full-res source `[0]` must become the **resolved** texture (bloom should bloom the stable image, not the jittered raw one).
- `git-craft/src/render/exposure.rs` `ExposurePass::set_input(device, hdr_view)` samples the HDR target for the histogram. After TAA, it should read the **resolved** texture (stable luminance).
- `git-craft/src/game/camera.rs` `view_proj(aspect)` = `perspective_rh(fov_y, aspect, 0.1, 800.0) * look_to_rh(...)`, wgpu depth 0..1.
- `FrameUniform` (terrain.rs, 208 B) holds `view_proj`, `inv_view_proj`, `camera`, `sky`, `sun`, `sun_color`, `params(xy=viewport, z=AP_KM_PER_METER)`. `FrameParams` feeds it. The terrain/sky main pass reads `view_proj` for rasterization and `inv_view_proj` (sky) — both must become the **jittered** matrices so the rasterized image is sub-pixel offset. Shadows sample by world position (jitter-independent); the frustum cull uses the unjittered matrix (cull must not wobble).
- wgpu 29: `immediate_size: 0` on pipeline layouts; `depth_slice: None` on colour attachments; `multiview_mask: None`; compute/render timestamp_writes optional.

## Frame-graph after this rung

```
luts(compute) → shadow0..2(render) → main(render, jittered VP, depth STORE)
   → taa(render/compute: resolve) → bloom → exposure → post → egui
```
New pass label `taa` is inserted between `main` and `bloom`.

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/render/taa.rs` | new | Halton jitter sequence (pure, TDD); `TaaPass` (resolve pipeline, history ping-pong, TAA uniform) |
| `assets/shaders/taa.wgsl` | new | Fullscreen resolve: reproject + neighborhood clamp + blend |
| `src/render/targets.rs` | modify | History ×2 + resolved HDR textures; depth made sampleable |
| `src/render/depth.rs` | modify | Depth texture gains `TEXTURE_BINDING`; return texture+view |
| `src/render/post.rs` | modify | HDR input ← resolved texture |
| `src/render/bloom.rs` | modify | Full-res source ← resolved texture |
| `src/render/exposure.rs` | modify | Histogram input ← resolved texture |
| `src/render/mod.rs` | modify | `pub mod taa;` |
| `src/app.rs` | modify | Jitter the main VP, track prev unjittered VP, wire TAA pass + label |

## Tasks

| Task | Lands |
|---|---|
| 1 | Halton sequence (TDD); sampleable depth + main-pass depth Store; history/resolved targets — no behavior change |
| 2 | Jitter main VP + TAA uniform + passthrough resolve wired into the graph (bloom/exposure/post read resolved) |
| 3 | Real resolve: reproject history + neighborhood clamp + blend — shimmer dies |
| 4 | Robustness (offscreen/disocclusion weight, resize, first frame), tuning, final review + playtest |

---

### Task 1: Halton jitter sequence + sampleable depth + history/resolved targets

**Files:** create `git-craft/src/render/taa.rs` (jitter half only); modify `git-craft/src/render/depth.rs`, `git-craft/src/render/targets.rs`, `git-craft/src/render/mod.rs`, `git-craft/src/app.rs`.

No behaviour change this task — the jitter values exist but are not yet applied; the new textures exist but are not yet sampled. Verify the app still renders identically.

- [ ] **Step 1: Failing tests for the jitter sequence**

Create `git-craft/src/render/taa.rs` with only:

```rust
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
        assert!(sx.abs() / JITTER_PERIOD as f32 < 0.2, "mean x {sx}");
        assert!(sy.abs() / JITTER_PERIOD as f32 < 0.2, "mean y {sy}");
    }
}
```

Register `pub mod taa;` in `src/render/mod.rs`.

- [ ] **Step 2: Run — expect FAIL** (`cargo test --manifest-path git-craft/Cargo.toml taa::`).

- [ ] **Step 3: The implementation is already in Step 1's non-test code** (halton, jitter_offset). Fix the placeholder typos, run the tests green.

- [ ] **Step 4: Make depth sampleable**

Rewrite `git-craft/src/render/depth.rs` so the texture carries `TEXTURE_BINDING` and the function returns both the texture and the view (the texture must outlive the view for a second sampling view; returning the view alone is fine if we keep usage). Minimal change — keep the single-view return but add the usage:

```rust
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        // TEXTURE_BINDING so the TAA resolve can sample depth for reprojection.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
```

- [ ] **Step 5: Main-pass depth Store**

In `app.rs` the main render pass depth attachment: change `store: wgpu::StoreOp::Discard` → `store: wgpu::StoreOp::Store` (depth is now sampled by TAA). Update the trailing comment to say so.

- [ ] **Step 6: History + resolved targets**

In `targets.rs` add to `RenderTargets`: `pub resolved_view: wgpu::TextureView` and `pub history_views: [wgpu::TextureView; 2]`, all `HDR_FORMAT`, full-res, usage `RENDER_ATTACHMENT | TEXTURE_BINDING`. Build them in `new()` next to the HDR texture. (They are unused until Task 2; that is fine — `RenderTargets` fields are `pub` and referenced by name, so no dead_code.)

- [ ] **Step 7: Gates + smoke-run** (`cargo test`, clippy, 20 s run). Image identical to current. Resize once — no crash.

- [ ] **Step 8: Commit**

```bash
git add git-craft/src/render/taa.rs git-craft/src/render/depth.rs git-craft/src/render/targets.rs git-craft/src/render/mod.rs git-craft/src/app.rs
git commit -m "feat: add jitter sequence, sampleable depth, and history targets for TAA (m5)"
```

---

### Task 2: Jitter the main pass + TAA uniform + passthrough resolve wired into the graph

**Files:** modify `taa.rs` (TaaPass + uniform), create `assets/shaders/taa.wgsl`, modify `post.rs`/`bloom.rs`/`exposure.rs` (input ← resolved), `app.rs`.

This task makes the main pass render jittered and routes everything downstream through a TAA resolve that, for now, just **passes the current frame through** (copies current HDR → resolved + writes history). The result will visibly shimmer/jitter (expected — the real accumulation is Task 3); the point is to validate the plumbing, the jittered projection, and that bloom/exposure/post read the resolved texture.

- [ ] **Step 1: TAA uniform + layout test**

In `taa.rs`:

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TaaUniform {
    /// Reconstruct world pos from this frame's depth (jittered inverse VP).
    pub inv_view_proj: [[f32; 4]; 4],
    /// Reproject world pos to last frame's screen (previous UNJITTERED VP).
    pub prev_view_proj: [[f32; 4]; 4],
    /// xy = viewport px; z = blend weight (current contribution); w = valid
    /// history flag (0 on the first frame / after resize).
    pub params: [f32; 4],
}
```

Layout test: size 144, `prev_view_proj` at 64, `params` at 128.

- [ ] **Step 2: `assets/shaders/taa.wgsl` (passthrough this task)**

```wgsl
// Temporal AA resolve. Task 2 passthrough: output = current; history = current.
// Task 3 fills in reproject + neighborhood clamp + blend.

struct TaaUniform {
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    params: vec4<f32>, // xy viewport px, z blend, w valid
};

@group(0) @binding(0) var current_tex: texture_2d<f32>;
@group(0) @binding(1) var history_tex: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var<uniform> u: TaaUniform;

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4(textureSampleLevel(current_tex, samp, in.uv, 0.0).rgb, 1.0);
}
```

- [ ] **Step 3: `TaaPass` in `taa.rs`**

`TaaPass::new(device, targets, src)`: a fullscreen pipeline writing `HDR_FORMAT`; bind group layout = current(0, tex) + history(1, tex) + depth(2, tex, `TextureSampleType::Float{filterable:false}` — Depth32Float sampled as float via a non-filtering view is awkward; instead sample depth with `textureLoad`. SIMPLER: bind depth as `Texture{ sample_type: Depth }`? Depth textures sampled in WGSL need either a comparison sampler or `textureLoad`. Use `textureLoad(depth_tex, vec2<i32>(pos.xy), 0)` in Task 3 and bind depth as `sample_type: Float{filterable:false}` with `texture_depth_2d`? — DECISION: bind depth as `texture_2d<f32>` with `TextureSampleType::Float{ filterable: false }` and read via `textureLoad` (integer coords, no sampler). This avoids depth-comparison-sampler complications.) current/history use a `Filtering` linear-clamp sampler (binding 3). uniform at binding 4.

Methods: `prepare(queue, &TaaUniform)`; `swap_shader`; `resolved_view()` accessor (returns the resolved texture this frame writes to — for Task 2 it always writes `targets.resolved_view`); `encode(encoder, targets, history_read_idx, timestamp_writes)` — renders into `targets.resolved_view` sampling `current=targets.hdr_view`, `history=targets.history_views[read_idx]`, `depth=depth_view`. **Also** copy resolved → `history_views[1-read_idx]` for next frame: simplest is a second draw (or `copy_texture_to_texture` from resolved to the write history). Use `encoder.copy_texture_to_texture(resolved, history_write, size)` — both are `HDR_FORMAT`, same size; resolved needs `COPY_SRC`, history needs `COPY_DST` usage (add to those textures in targets.rs). The app ping-pongs `history_read_idx` each frame.

Pass the depth view into `encode` (the app owns it).

- [ ] **Step 4: Downstream inputs ← resolved**

- `post.rs`: `set_input`/`new` HDR arg already exists; the app will pass `targets.resolved_view` instead of `targets.hdr_view`.
- `bloom.rs` `set_targets`: change source `[0]` from `targets.hdr_view` to `targets.resolved_view`.
- `exposure.rs`: the app passes `targets.resolved_view` to `set_input`.

(Where the texture is chosen inside a renderer from `&targets`, switch the field; where it's an argument, the app switches the argument.)

- [ ] **Step 5: Jitter + wire in `app.rs`**

- Fields: `taa: Option<TaaPass>`, `prev_view_proj: glam::Mat4` (init `Mat4::IDENTITY`), `taa_history_idx: usize` (0), `frame_index: u64` (0), `taa_valid: f32` (0.0 — set to 0 on resize/first frame).
- Each frame, after computing the unjittered `view_proj`:

```rust
self.frame_index += 1;
let (jx, jy) = crate::render::taa::jitter_offset(self.frame_index);
let (w, h) = (gpu.config.width as f32, gpu.config.height as f32);
// NDC sub-pixel offset (one pixel = 2/size in NDC).
let jitter = glam::Mat4::from_translation(glam::vec3(2.0 * jx / w, 2.0 * jy / h, 0.0));
let jittered_vp = jitter * view_proj;
```

Use **`jittered_vp`** for `terrain.write_frame` (FrameParams.view_proj) and anything feeding the main-pass rasterization (sky reads the FrameUniform). Keep the **unjittered** `view_proj` for: frustum cull, shadow `prepare`, sky-LUT `AtmUniform.inv_view_proj` (twilight is smooth; either works, use unjittered for stability), and reprojection bookkeeping.

- Pass labels: insert `taa` after `main`: `["luts","shadow0","shadow1","shadow2","main","taa","bloom","exposure","post"]`; `PASS_TAA=5`, shift `PASS_BLOOM=6`, `PASS_EXPOSURE=7`, `PASS_POST=8`. Update every `PASS_*` const + use.
- Build `TaaPass` in `resumed()`; watch `"taa"`; hot-reload arm.
- TAA uniform per frame: `inv_view_proj = jittered_vp.inverse()`, `prev_view_proj = self.prev_view_proj` (last frame's UNJITTERED vp), `params = [w, h, BLEND, self.taa_valid]` (BLEND const e.g. 0.1; Task 2 passthrough ignores them).
- Encode order: after the main pass block, `taa.encode(&mut encoder, targets, self.taa_history_idx, taa_writes)`; then bloom/exposure/post (already reading resolved). After submit: `self.prev_view_proj = view_proj;` (unjittered), `self.taa_history_idx ^= 1;`, `self.taa_valid = 1.0;`.
- Resize: recreate targets, rebuild TAA bind groups (it samples targets), set `self.taa_valid = 0.0`, and re-`set_input` post/bloom/exposure to the new resolved view.

- [ ] **Step 6: Gates + smoke-run.** Expect a **jittering/shimmering** image (passthrough — no accumulation yet). Confirm: no validation errors, bloom/exposure/post visibly still work (sky, shadows, tonemap present), HUD shows a `taa` pass time. The shimmer is expected and goes away in Task 3.

- [ ] **Step 7: Commit**

```bash
git add git-craft/src/render/taa.rs git-craft/assets/shaders/taa.wgsl git-craft/src/render/post.rs git-craft/src/render/bloom.rs git-craft/src/render/exposure.rs git-craft/src/render/targets.rs git-craft/src/render/mod.rs git-craft/src/app.rs
git commit -m "feat: jitter the main pass and route it through a passthrough TAA resolve (m5)"
```

---

### Task 3: Real resolve — reproject history, neighborhood clamp, blend

**Files:** `assets/shaders/taa.wgsl` (the resolve math). Possibly `taa.rs` if a constant moves.

This is where the shimmer dies. Pure shader change (hot-reloadable to tune).

- [ ] **Step 1: Replace `fs_main` in `taa.wgsl`** with the accumulating resolve:

```wgsl
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.pos.xy);
    let current = textureLoad(current_tex, px, 0).rgb;

    // Reconstruct world position from this frame's (jittered) depth.
    let depth = textureLoad(depth_tex, px, 0).r;
    let ndc = vec3(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, depth);
    let world_h = u.inv_view_proj * vec4(ndc, 1.0);
    let world = world_h.xyz / world_h.w;

    // Reproject to last frame's screen (unjittered) to find the history texel.
    let prev_clip = u.prev_view_proj * vec4(world, 1.0);
    let prev_ndc = prev_clip.xyz / prev_clip.w;
    let prev_uv = vec2(prev_ndc.x * 0.5 + 0.5, -prev_ndc.y * 0.5 + 0.5);

    let on_screen = all(prev_uv >= vec2(0.0)) && all(prev_uv <= vec2(1.0));
    if u.params.w < 0.5 || !on_screen || depth >= 1.0 {
        // First frame / disoccluded / sky pixel: take current, no history.
        return vec4(current, 1.0);
    }

    var history = textureSampleLevel(history_tex, samp, prev_uv, 0.0).rgb;

    // Neighborhood AABB clamp (kills ghosting): bound history to the colour
    // box of the current 3x3 neighborhood.
    var lo = current;
    var hi = current;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let c = textureLoad(current_tex, px + vec2(dx, dy), 0).rgb;
            lo = min(lo, c);
            hi = max(hi, c);
        }
    }
    history = clamp(history, lo, hi);

    let blend = u.params.z; // current contribution (~0.1)
    return vec4(mix(history, current, blend), 1.0);
}
```

- [ ] **Step 2: Gates** (`cargo test` — the WGSL glob validity test re-checks taa.wgsl; clippy).

- [ ] **Step 3: Smoke-run — the moment of truth.** Walk and fly: edges and shadow boundaries should be stable, no per-pixel shimmer. Watch for: ghosting/trails behind the camera motion (if so, the clamp is too loose — Task 4 tightens), over-blur (blend too low), or smearing on disocclusion (Task 4). Note what you see for Task 4.

- [ ] **Step 4: Commit**

```bash
git add git-craft/assets/shaders/taa.wgsl
git commit -m "feat: accumulate TAA history with reprojection and neighborhood clamp (m5)"
```

---

### Task 4: Robustness, tuning, final review + playtest

**Files:** `taa.wgsl` (tuning/edge cases), maybe `taa.rs`/`app.rs`.

- [ ] **Step 1: Edge-case hardening in `taa.wgsl`** as the smoke-test of Task 3 indicates. Apply whichever are needed:
  - **Disocclusion**: when reprojection lands far from where geometry now is, fall back to current. A robust proxy without a velocity buffer: if `clamp(history,lo,hi)` moved the history a lot (luminance distance between raw history and clamped > threshold), raise the current blend toward 1.0. Implement as `let ghost = clamp01(length(history_raw - history_clamped) * K); blend = mix(blend, 1.0, ghost);`.
  - **Sky / far plane**: pixels with `depth >= 1.0` always take current (already handled) — keep, the sky is smooth so no AA needed and reprojecting the far plane is unstable.
  - **History clamp variant**: if minmax AABB still ghosts, switch to a mean±N·stddev "variance clip" over the 3×3 (compute mean and mean-of-squares, σ = sqrt(max(0, m2 - m*m)), clamp to `[mean - 1.25σ, mean + 1.25σ]`). Pick whichever looks cleaner; keep only one.
  - **Blend weight**: tune `BLEND` (start 0.1). Lower = steadier but more ghosting/lag; higher = sharper but more shimmer. Land the best-looking value; hot-reload to find it, then bake into the `params.z` the app writes.

- [ ] **Step 2: Resize robustness.** Confirm resizing mid-run does not smear (taa_valid=0 on resize discards stale history; history textures recreated). Fix if needed in app.rs resize.

- [ ] **Step 3: Full gates on a clean tree** (`git status` empty, `cargo test`, clippy).

- [ ] **Step 4: Final cross-cutting review** (dispatch a reviewer over `git diff` of the TAA commits): jitter applied only to the main-pass VP (not cull/shadows); prev_view_proj is the UNJITTERED previous frame; history ping-pong indices correct (no read==write same texture in a pass); depth Store + sampleable; all `PASS_*` indices consistent; targets recreated + bind groups rebuilt on resize; no validation errors in a 60 s run.

- [ ] **Step 5: Playtest hand-off.** Long run (60–90 s). Report to the user: shimmer gone under walk + fly; no ghosting trails; shadows stable; FPS cost of the resolve (HUD `taa` ms) and net FPS vs pre-TAA. Note that render-scale upsampling (0.75× + TAA) remains deferred — a future M5b task — and the GTAO/volumetrics/water rungs can now build on the jitter+history infra this rung established.

---

## Self-review

**Spec coverage:** delivers §6's "TAA (not MSAA)" with sub-pixel jitter + temporal accumulation; establishes the jitter+history infrastructure §6 says GTAO and volumetrics depend on. **Deferred (stated):** render-scale 0.75× upsampling (§11 safety valve) — a later M5b task.

**Seams an implementer must respect:**
- Jitter goes ONLY on the main-pass rasterization VP (terrain + sky). Frustum cull and shadow fitting use the UNJITTERED VP or they wobble.
- `prev_view_proj` written to the TAA uniform must be the UNJITTERED VP of the PREVIOUS frame; update `self.prev_view_proj = view_proj` (unjittered) only once per frame after encoding.
- History is ping-pong: the resolve reads `history_views[idx]` and the next frame reads `history_views[idx^1]`; never bind the same texture as the pass's render target and its sampled history. The Task-2 design writes resolved into `resolved_view` and copies to the write-history, so resolve never reads and writes one texture.
- Depth must be `Store` + `TEXTURE_BINDING`; sample it with `textureLoad` (integer coords), not a filtering sampler.
- `taa_valid=0` on the first frame and after every resize, or the first blend reads garbage/stale history.
- All `PASS_*` indices shift when `taa` is inserted — update them together.
