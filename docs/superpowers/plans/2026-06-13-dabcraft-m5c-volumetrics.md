---
title: dabcraft M5c — Froxel volumetrics (god rays + height fog)
date: 2026-06-13
domain: render-layer
type: enhancement
priority: high
breaking: false
db-migration: false
rls-affecting: false
slice: null
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files: [dabcraft/src/render/volumetric.rs, dabcraft/src/render/targets.rs, dabcraft/src/render/gtao.rs, dabcraft/src/render/shadow.rs, dabcraft/src/app.rs, dabcraft/assets/shaders/volumetric.wgsl, dabcraft/assets/shaders/composite.wgsl]
trigger-tasks-touched: []
shared-modules-touched: [dabcraft/src/render/targets.rs, dabcraft/src/app.rs]
---

# dabcraft M5c — Froxel Volumetrics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the M5 "froxel volumetrics" rung — a view-frustum-aligned 3D froxel grid that produces **god rays** (crepuscular shafts where terrain occludes the sun, sampled against the existing CSM) and **height-based fog**, temporally denoised, composited into the HDR scene upstream of TAA.

**Architecture:** Two compute passes build a fixed-resolution froxel grid (`160×90×64`, `Rgba16Float`) covering the camera frustum from `VOL_NEAR` to `VOL_FAR` with an exponential depth distribution (near slices dense):

1. `cs_inscatter` — per froxel: reconstruct the world position from `(uv, slice)`, evaluate fog density (height falloff + uniform haze), and compute single-scatter radiance = `sun_color · shadowCSM(world_pos) · HG_phase(view·sun)` + `sky_ambient · isotropic`. A per-pixel + per-frame jitter offsets the sample depth within the slice; the grid is **temporally reprojected** against the previous frame's grid (ping-pong) to denoise the 1-sample-per-froxel estimate. Output: `rgb = in-scattered radiance × density`, `a = extinction σ_e`.
2. `cs_integrate` — front-to-back accumulation along +Z per froxel column: `accum += transmittance · energy_conserving_integral(s, σ_e, Δt)`, `transmittance *= exp(-σ_e·Δt)`. Output: `rgb = accumulated in-scatter to this slice`, `a = transmittance to this slice`.

**Application:** The existing **composite** pass (already full-res, depth-aware, upstream of TAA) gains a froxel sample: `color = color · fog.transmittance + fog.inscatter`, where the froxel is sampled at `(screen_uv, view_dist→slice)`. Reusing composite keeps the frame graph flat and lets the downstream TAA stabilize residual god-ray noise (spec §6 puts volumetrics' composite at pass 6, exactly here).

**Tech Stack:** Rust 2024, wgpu 29, WGSL. Compute passes for the grid (3D storage textures, mirrors the existing sky-LUT/aerial pattern); the existing fullscreen render passes do the application.

**Frame graph after this rung:** `luts → shadow0..2 → main(hdr+gbuf, jittered) → gtao(half) → gtao_blur(half) → vol_inscatter(160×90×64) → vol_integrate → composite(full: AO + volumetrics) → taa → bloom → exposure → post → egui`.

**Spec alignment & deviations (shipped code is authoritative):**
- Spec §6 pass 4: "froxel volumetric scatter/integrate (samples CSM + flood-fill skylight, temporal jitter)". We sample the **CSM** for the directional shaft term and use the analytic **sky-ambient color** for the isotropic term. We do *not* sample flood-fill skylight in the froxel pass — flood-fill light is baked per-vertex into meshes, not available as a world-space field at froxel centers; the analytic sky ambient is the practical stand-in and matches what the opaque shader already uses for its ambient term. Underground god rays are therefore governed by the CSM `splits`/range exactly like the opaque shadow term (caves past the cascades get no shaft, consistent with the opaque pass).
- The grid is **fixed resolution** (screen-independent, like the 32³ aerial LUT) — composite samples it by continuous screen uv, so no resize rebuild of the grid is needed.
- Application lives in the **composite** pass (not the transparent/water pass, which does not exist until the next rung); spec §6 lists "composite volumetrics" at pass 6, so this is spec-faithful.

**Validation:** Rendering correctness via the F3 HUD per-pass GPU timers + visual inspection (project convention: rendering is validated via HUD/`--bench`, never "by feel"). Pure helpers (froxel slice↔view-distance mapping, uniform layout sizes, HG phase normalization) are unit-tested. Quality gates per task: `cargo test` + `cargo clippy --all-targets -- -D warnings`. `cargo fmt` is NOT a gate.

**Environment:** Rust via rustup — every shell needs `export PATH="$HOME/.cargo/bin:$PATH"` before `cargo`. Run cargo with `--manifest-path dabcraft/Cargo.toml` from the repo root (`/Users/onurtellioglu/Github/Minecraft`). No git remote — skip push/PR. Branch is already `feat/m5-shaders` (M5 merges as one milestone; do NOT merge to main in this rung).

---

## Froxel math (single source of truth)

Grid: `VOL_W=160`, `VOL_H=90`, `VOL_D=64`. `VOL_NEAR=0.5`, `VOL_FAR=360.0` (= `SHADOW_FAR`; beyond the cascades the shaft term is 1.0 anyway).

Exponential depth so near precision is high. For slice index `z ∈ [0, VOL_D)`:

```
slice_to_view_dist(z) = VOL_NEAR * pow(VOL_FAR / VOL_NEAR, z / VOL_D)      // slice front edge
view_dist_to_slice(d) = VOL_D * log(d / VOL_NEAR) / log(VOL_FAR / VOL_NEAR) // inverse, for sampling
```

World position of a froxel center (uv = `(x+0.5)/VOL_W`, `(y+0.5)/VOL_H`; depth jittered within the slice):

```
view_dist = slice_to_view_dist(z + jitter)              // jitter ∈ [0,1)
ndc  = vec4(uv.x*2-1, 1-uv.y*2, 0.5, 1)
ray  = normalize((inv_view_proj · ndc).xyz/w − camera)   // unjittered camera VP
world_pos = camera + ray * view_dist
```

Density (valleys pool fog): `density = VOL_DENSITY * (VOL_HAZE + exp(-max(0, world_pos.y − VOL_FOG_Y0)/VOL_FOG_H))`.
Scattering `σ_s = density`, extinction `σ_e = max(density + VOL_ABSORB·density, 1e-5)`.
Phase: Henyey-Greenstein `hg(cosθ, g) = (1−g²) / (4π·(1+g²−2g·cosθ)^1.5)`, `g = VOL_HG_G`, `cosθ = dot(ray, sun_dir)`.
In-scatter: `s = σ_s · (sun_color · shadowCSM(world_pos) · hg + sky_color · VOL_AMBIENT)`.

Integrate (Frostbite energy-conserving, per slice of thickness `Δt`):

```
t_slice = exp(-σ_e · Δt)
s_int   = (s − s · t_slice) / σ_e
accum  += transmittance · s_int
transmittance *= t_slice
```

Apply in composite: `fog = froxel_integrated(screen_uv, view_dist_to_slice(view_dist)/VOL_D)`; `color = color · fog.a + fog.rgb`.

---

## File Structure

- `dabcraft/src/render/targets.rs` — **modify**: add `FROXEL_FORMAT`, `VOL_W/H/D` consts, `froxel_scatter_view`, `froxel_integrated_view` (+ a ping-pong history view in Task 3). Fixed-size 3D textures, NOT recreated on resize.
- `dabcraft/src/render/volumetric.rs` — **create**: `VolumetricPass` (inscatter + integrate compute pipelines, `VolUniform`), pure helpers `slice_to_view_dist`/`view_dist_to_slice` + a `hg_phase` test hook.
- `dabcraft/assets/shaders/volumetric.wgsl` — **create**: `cs_inscatter` (CSM god ray + height fog) and `cs_integrate` (front-to-back).
- `dabcraft/src/render/shadow.rs` — **modify**: expose `uniform_buffer()`/`array_view()` to the volumetric bind group (already pub; add a comparison-sampler accessor or let VolumetricPass make its own).
- `dabcraft/assets/shaders/composite.wgsl` — **modify**: sample the integrated froxel grid and apply `color·t + inscatter`.
- `dabcraft/src/render/gtao.rs` — **modify**: `CompositePass` gains the froxel texture + sampler + a small uniform (inv_view_proj, camera, viewport, VOL params) in its bind group.
- `dabcraft/src/app.rs` — **modify**: construct `VolumetricPass`; insert `vol_inscatter`/`vol_integrate` encode after the GTAO blur; renumber `PASS_*` (+`PASS_VOLUMETRIC`); feed the froxel grid + a vol uniform to composite each frame; register `volumetric.wgsl` for hot-reload; resize is a no-op for the fixed grid.

---

### Task 1: Frustum froxel grid — in-scatter + integrate compute passes

**Files:** create `volumetric.rs`, `volumetric.wgsl`; modify `targets.rs`, `app.rs`.

The grid is two fixed `160×90×64` `Rgba16Float` 3D textures: `froxel_scatter` (write of `cs_inscatter`, read of `cs_integrate`) and `froxel_integrated` (write of `cs_integrate`, read of composite in Task 2). Build them in `RenderTargets::new` but leave them out of the resize path (fixed size).

`VolUniform` (mirror in WGSL, test the size): `inv_view_proj: mat4 (64)`, `camera: vec4 (xyz pos, w frame_index)`, `sun: vec4 (xyz dir, w 1=sun/0=moon)`, `sun_color: vec4`, `sky: vec4 (rgb ambient, w unused)`. = 128 B.

`cs_inscatter` (`@workgroup_size(4,4,4)`): guard `id < (160,90,64)`; compute `world_pos` per the froxel math (jitter = IGN(id.xy, frame_index)); `shadowCSM(world_pos)` reuses terrain's cascade-select + `textureSampleCompareLevel` (works in compute — no derivatives); write `vec4(s, σ_e)`. **Temporal reproject is added in Task 3** — Task 1 writes the raw single-sample estimate.

`cs_integrate` (`@workgroup_size(8,8,1)` over the 160×90 columns): loop `z` 0→63, accumulate per the integrate math, `textureStore(froxel_integrated, vec3(x,y,z), vec4(accum, transmittance))`.

`VolumetricPass`: bind group 0 = `VolUniform` + shadow uniform + shadow `texture_depth_2d_array` + comparison sampler; group 1 = scatter storage (write) for inscatter / scatter sampled + integrated storage (write) for integrate. Two pipelines, `encode(encoder, timestamp_writes)` runs inscatter then integrate in one compute pass (like `SkyLuts::encode`). `prepare(queue, &VolUniform)`. `swap_shader`. Pure helpers `slice_to_view_dist`/`view_dist_to_slice` (+ roundtrip test) and `hg_phase` (+ a "integrates toward 1 over the sphere" sanity test) live module-level for TDD.

- [ ] **Step 1:** `targets.rs` — add `FROXEL_FORMAT = Rgba16Float`, `VOL_W=160/VOL_H=90/VOL_D=64`, create both 3D textures in `new`, expose `froxel_scatter_view`/`froxel_integrated_view`. Test: a `vol_grid_dims` assertion. Do NOT touch resize.
- [ ] **Step 2:** `volumetric.wgsl` — write `cs_inscatter` + `cs_integrate` + shared helpers (slice mapping, HG phase, CSM sample copied from terrain.wgsl `shadow_factor` minus the normal-offset bias, which froxels don't have). No temporal term yet.
- [ ] **Step 3:** `volumetric.rs` — `VolumetricPass`, `VolUniform` (+128-byte size test), pure helpers + tests. Mirror `SkyLuts` bind-group/pipeline construction.
- [ ] **Step 4:** `app.rs` — build `VolumetricPass` in setup (after shadow + targets exist); add `PASS_VOLUMETRIC`, renumber `PASS_*` as one block, grow `PASS_LABELS`; `prepare` the vol uniform each frame (unjittered VP, same `light_dir/light_color/sky_color` as terrain); encode after the GTAO blur, before composite; register for hot-reload + `swap_shader`. Composite application is Task 2 — here just build the grid and time it.
- [ ] **Step 5:** Build + test: `cargo test` + `cargo clippy --all-targets -- -D warnings`. Expected: green; a "volumetric" timer appears in the HUD with the grid built but not yet visible on screen.
- [ ] **Step 6:** Commit: `feat: build a frustum froxel grid with CSM god-ray in-scatter (m5)`.

---

### Task 2: Composite the volumetric fog + god rays onto the scene

**Files:** modify `composite.wgsl`, `gtao.rs` (`CompositePass`), `app.rs`.

The composite pass currently does `color = hdr × (1 − ambientWeight·(1−ao))`. Add: sample the integrated froxel grid at `(screen_uv, view_dist_to_slice(view_dist)/VOL_D)` and apply `color = color · fog.a + fog.rgb` **after** the AO multiply. Reconstruct `view_dist` from the (jittered) depth buffer + `inv_view_proj` (composite already binds depth for the gbuf/AO path; if not, add a depth binding).

- [ ] **Step 1:** `gtao.rs` `CompositePass` — extend the bind group with the `froxel_integrated` texture (3D, filterable), a linear clamp sampler, and a `CompUniform` grown to carry `inv_view_proj`, `camera`, `viewport`, and `[VOL_NEAR, VOL_FAR, VOL_D, debug_flag]`. Update its size test.
- [ ] **Step 2:** `composite.wgsl` — reconstruct `view_dist`; `view_dist_to_slice` (same formula as WGSL/Rust); sample froxel; apply `color·t + inscatter`. Sky pixels (`depth >= 1.0`) sample the far slice so distant haze still applies up to `VOL_FAR`, but the sky itself (depth==1) uses `view_dist = VOL_FAR` clamp.
- [ ] **Step 3:** `app.rs` — feed the froxel grid view + the grown composite uniform each frame (reuse the vol uniform's matrices). Resize rebuilds the composite bind group (froxel views are stable, but the bind group is recreated with the rest).
- [ ] **Step 4:** Build + test + clippy. HUD-validate: at a low sun angle, shafts appear through terrain gaps; fog thickens in valleys; `min(total) ≤ 8.3 ms` recorded. Confirm god rays are stable through TAA (some residual shimmer expected pre-Task-3).
- [ ] **Step 5:** Commit: `feat: composite volumetric fog and god rays onto the scene (m5)`.

---

### Task 3: Temporal reprojection, V-key debug view, and constant tuning

**Files:** modify `targets.rs` (history grid), `volumetric.wgsl`, `volumetric.rs`, `app.rs`, `composite.wgsl`/`gtao.rs` (debug flag).

Single-sample froxels boil under motion. Add a **ping-pong history grid**: `cs_inscatter` reads the previous frame's *scatter* grid at the reprojected uvw of the current froxel's `world_pos` under `prev_view_proj`, and blends `curr = mix(history, curr, VOL_TAA_ALPHA)` when the reprojected uvw is in-bounds (else `alpha = 1`). Two scatter textures ping-ponged by frame parity.

- [ ] **Step 1:** `targets.rs` — add a second scatter texture; expose `froxel_scatter_views: [TextureView; 2]`. `volumetric.rs` builds two inscatter bind groups (read prev `[1-i]`, write curr `[i]`) and two integrate bind groups (read curr `[i]`), selected by `taa_history_idx`-style parity passed to `encode`.
- [ ] **Step 2:** `volumetric.wgsl` — add the reproject + blend in `cs_inscatter` (needs `prev_view_proj` in `VolUniform`; grow it + bump the size test). Guard the first frame / post-resize with an `alpha = 1` validity flag.
- [ ] **Step 3:** `app.rs` — store/advance `prev_view_proj` (already kept for TAA — reuse it), pass the froxel parity index to `vol.encode`.
- [ ] **Step 4:** V-key debug: `vol_debug: bool` in `app.rs`, toggled on `KeyCode::KeyV`, pushed into the composite `debug_flag`; `composite.wgsl` shows the raw in-scatter (`fog.rgb`) full-screen when set. (Mirrors the GTAO G-key.)
- [ ] **Step 5:** Name + tune the `VOL_*` constants in `app.rs` (one block, documented like the `GTAO_*` consts): `VOL_DENSITY`, `VOL_HAZE`, `VOL_FOG_Y0`, `VOL_FOG_H`, `VOL_ABSORB`, `VOL_HG_G`, `VOL_AMBIENT`, `VOL_TAA_ALPHA`.
- [ ] **Step 6:** Build + test + clippy. HUD-validate: god rays denoise (no boiling) under camera motion + day-cycle sweep; V shows the raw scatter; record `volumetric`/`composite` ms + total in the commit body.
- [ ] **Step 7:** Commit(s): `feat: temporally reproject the froxel grid to denoise god rays (m5)` then `feat: add a V-key volumetric debug view and name the fog constants (m5)`.

---

## Self-Review

**Spec coverage (§6 pass 4/5/6):**
- "froxel volumetric scatter/integrate" → Task 1 `cs_inscatter` + `cs_integrate`. ✓
- "samples CSM" → Task 1 CSM god-ray term via `textureSampleCompareLevel`. ✓ ("flood-fill skylight" deviation documented above — analytic sky ambient stands in.)
- "temporal jitter" → Task 1 per-froxel IGN depth jitter; Task 3 temporal reprojection denoise. ✓
- "composite volumetrics" (pass 6) → Task 2 applies in the composite pass, upstream of TAA. ✓
- "wgpu timestamp queries measure every pass" → Task 1 adds `PASS_VOLUMETRIC`; Tasks 2/4 HUD-validate the budget. ✓

**Seams respected (from M5a/TAA/GTAO reviews):**
- All `PASS_*` consts renumbered together; `PASS_LABELS` length matches `GpuTimer::new(PASS_LABELS)`.
- Jitter isolation: the froxel grid uses the **unjittered** camera VP (it is world-space, sampled by composite which itself uses the jittered depth for the *pixel* world pos — the grid stays jitter-free, consistent with sky/shadow LUTs). Composite reads jittered depth exactly as it already does for AO.
- Fixed-size grid → resize touches nothing in `targets.rs` for the froxel textures; only the composite bind group rebuilds (already rebuilt on resize for AO).
- New shader registered for hot-reload + given `swap_shader`, matching the taa/gtao pattern.
- CSM is read-only here; shadow.rs gains only accessors. Temporal reproject reuses `prev_view_proj` already maintained for TAA.

