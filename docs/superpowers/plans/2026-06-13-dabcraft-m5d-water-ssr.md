---
title: dabcraft M5d — Reflective/refractive water (SSR)
date: 2026-06-13
domain: render-layer
type: enhancement
priority: high
breaking: false
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files: [dabcraft/src/mesh/greedy.rs, dabcraft/src/world/jobs.rs, dabcraft/src/render/terrain.rs, dabcraft/src/render/water.rs, dabcraft/src/render/targets.rs, dabcraft/src/app.rs, dabcraft/assets/shaders/water.wgsl]
---

# dabcraft M5d — Water SSR Implementation Plan

**Goal:** The final M5 ladder rung — turn water from an opaque blue solid into a transparent surface with **refraction** (the seafloor seen through the surface), **reflection** (screen-space ray-marched terrain reflections with a sky-view LUT fallback), **fresnel** blend, and depth-based water fog. Rendered in a dedicated transparent pass after the opaque/composited scene, fed through TAA.

**Why water was opaque:** M2 meshed water as a normal solid (`is_solid()` true), so it draws in the opaque terrain pass and the seafloor face under it is culled (solid→solid). Transparency needs (a) the seafloor emitted (opaque mesh must treat water as empty) and (b) water surface geometry in a separate draw so it can shade against the already-rendered scene.

**Architecture:**
1. **Meshing split** — `Mesher::mesh_layers(padded) -> (opaque, water)`. Opaque uses occupancy `is_solid && !water` (emits the seafloor now exposed to "air"); water uses occupancy `is_solid` but keeps only `WATER`-owned faces (i.e. water→air boundaries; water→solid stays culled). The existing `mesh()` is kept (delegates to a shared `mesh_with(occupancy, keep)`), so all mesher tests are unchanged.
2. **Dual arena ranges** — water quads share the *same* `quads_buffer`/`Arena`/`index_buffer` as opaque (vertex-pulling is identical); each resident section tracks a second `(water_offset, water_len)`. A separate `water_indirect` buffer holds per-visible-section water draw args.
3. **Transparent pass** — after `composite` (so the scene incl. AO + volumetrics is final) and before `taa`: copy `composited_view → scene_color`, then draw water into `composited_view` (LoadOp::Load, depth read-only LessEqual, depth-write OFF). The water shader samples `scene_color` for refraction/SSR source, the depth buffer for SSR + fog, the sky-view LUT for the reflection fallback, and writes the final blended water color (BlendState::REPLACE — refraction is done in-shader, no hardware blend). TAA then reads `composited_view` with water in it.
4. **`water.rs` / `WaterRenderer`** — owns the water pipeline + a `WaterUniform` (inv_view_proj, camera, sun, viewport, time, tuning) + the water indirect buffer; reuses terrain's camera/quads bind groups. SSR raymarch + fresnel + refraction + fog in `water.wgsl`.

**Frame graph after this rung:** `… → composite(7) → [copy→scene_color] → water(8) → taa(9) → bloom(10) → exposure(11) → post(12) → egui`. PASS_* renumber together; add `PASS_WATER`.

**Validation:** mesher split is unit-tested (water separated; seafloor emitted; existing tests intact); rendering via F3 HUD timers + visual. Gates: `cargo test` + `cargo clippy --all-targets -D warnings`.

**Environment:** `export PATH="$HOME/.cargo/bin:$PATH"`; `--manifest-path dabcraft/Cargo.toml`; branch `feat/m5-shaders` (no merge to main this rung); no git remote.

---

## Stage A — meshing split + transparent reflective/refractive water (no SSR yet)

- [ ] **A1 `greedy.rs`:** refactor `build_axis_cols`/`build_planes`/`ao_neighborhood` to take an `occupancy: impl Fn(BlockId)->bool`; `build_planes` also takes `keep: impl Fn(BlockId)->bool` (skip emitting faces whose owner fails `keep`). `mesh()` = `mesh_with(is_solid, |_| true)` (unchanged). Add `mesh_layers()` returning `(opaque, water)`. Tests: water surface separated into the water list; a sand-under-water column emits a seafloor top face in the opaque list; `mesh()` output unchanged for an all-stone scene.
- [ ] **A2 `jobs.rs`:** `JobResult::Meshed` carries `opaque: Vec<PackedQuad>` + `water: Vec<PackedQuad>` (rename `quads`); `spawn_mesh` calls `mesh_layers`.
- [ ] **A3 `terrain.rs`:** `SectionEntry` gains `water_offset/water_len`; upload allocates both ranges from the arena and writes both to `quads_buffer`; add `water_indirect` buffer + `write_water_indirect(queue, frustum, ...)` mirroring the opaque indirect writer; expose `quads_bind_group()`/`camera_bind_group()`/`index_buffer()`/a water-draw method. Free both ranges on eviction. Update arena-usage accounting.
- [ ] **A4 `targets.rs`:** add `scene_color` (HDR, COPY_DST + TEXTURE_BINDING, full offscreen size) recreated on resize/scale.
- [ ] **A5 `water.rs` + `water.wgsl`:** `WaterRenderer` (pipeline: vertex-pull from quads buffer like terrain's vertex stage, fragment = water shading; transparent pass, depth read-only). `WaterUniform`. Shader: reconstruct world pos, flat-up normal + cheap time ripple, fresnel(NdotV), refraction = sample `scene_color` at a normal-offset uv, reflection = sky-view LUT in the reflect dir, water tint + depth fog from `(water_depth − scene_depth)`. Output `mix(refract, reflect, fresnel)` shaded.
- [ ] **A6 `app.rs`:** build `WaterRenderer`; copy `composited_view→scene_color`; encode the water pass after composite, before TAA; add `PASS_WATER` (renumber all); feed the water uniform (time from `day`/frame); rebuild on resize/scale. Boot-check, commit: `feat: render transparent refractive water with a sky reflection (m5)`.

## Stage B — screen-space reflections

- [ ] **B1 `water.wgsl`:** replace the sky-only reflection with an SSR raymarch — march the reflection ray in view/screen space against the depth buffer (linear steps + a binary refine), sample `scene_color` at the hit; on miss or off-screen, fall back to the sky-view LUT. Fade SSR at screen edges and by ray length. Keep fresnel/refraction/fog from Stage A.
- [ ] **B2 tuning + commit:** name the water constants (tint, fog density, ripple, SSR steps/thickness, fresnel F0); HUD-validate the `water` timer + visuals; commit: `feat: screen-space reflections on water with a sky-LUT fallback (m5)`.

---

## Self-Review
- Spec §6 pass 5 "Transparent: water (SSR + refraction from sampled opaque color, fallback to sky-view LUT on SSR miss)" → Stage A refraction + Stage B SSR + sky fallback. ✓
- Spec §5 "Transparent geometry (water…) goes into a separate arena drawn in the transparent pass" → water shares the buffer but is tracked + drawn separately (a separate indirect + pipeline); functionally the separate-draw the spec wants, without duplicating the 32 MiB buffer (documented deviation).
- TBDR: water is a single forward transparent pass after opaque, depth read-only, no extra G-buffer. ✓
- Seams: PASS_* renumber together; scene_color recreated on resize+scale; water pipeline registered for hot-reload + swap_shader; water indirect rebuilt per frame like the opaque/shadow indirects.
