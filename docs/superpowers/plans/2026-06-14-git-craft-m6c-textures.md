---
title: git-craft M6c — procedural material textures (normal/roughness)
date: 2026-06-14
domain: render-layer
type: enhancement
priority: high
breaking: false
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files: [git-craft/src/render/material.rs, git-craft/src/render/terrain.rs, git-craft/src/render/mod.rs, git-craft/assets/shaders/terrain.wgsl, git-craft/src/app.rs, git-craft/CHANGELOG.md, git-craft/AGENTS.md]
---

# git-craft M6c — procedural material textures (normal/roughness)

**Goal:** Replace the flat per-block palette color (terrain.wgsl: *"M2 palette … procedural
textures replace this in M6"*) with **procedurally-generated material textures** — per-block
albedo detail, a tangent-space **normal map**, and a **roughness** channel — sampled in the
terrain pass and fed into the lighting (normal-mapped diffuse + a roughness-controlled specular
highlight). Surfaces gain texture and subtle relief instead of reading as flat-shaded solids.

**Why procedural, not image assets:** the project ships **no proprietary art** (guardrail) and
no texture files exist. Generating the materials in code from a deterministic hash keeps every
texel original and licensing-clean, and stays faithful to each block's existing base color
(reused from `block.color()`), so the world looks like itself — just textured.

**Infrastructure already in place:** the greedy mesher already packs the block id into the quad's
`texture` field (`greedy.rs: texture: block as u32`), and the shader has per-face `FACE_U`/`FACE_V`
/`FACE_NORMAL` axes (the tangent basis) and per-corner UVs. So no mesher change is needed — only a
material texture array indexed by block id, plus shader sampling + lighting.

**Architecture:**
1. **`src/render/material.rs` — pure atlas generation (TDD).** `build_atlas(size) -> MaterialAtlas`
   produces, per block id (0..=12), an **albedo+roughness** layer (RGB = `block.color()` modulated
   by deterministic value-noise detail, A = roughness) and a **normal** layer (tangent-space normal
   derived from a per-block height field via central differences). A CPU box-filter builds the full
   mip chain (terrain tiles to 384 m; mips kill the shimmer). Output is laid out mip-major,
   layer-major for direct `write_texture` upload. No I/O, no wgpu.
2. **`terrain.rs` — material bind group (group 4).** Build two `texture_2d_array` textures
   (albedo+roughness, normal; `Rgba8Unorm`, linear — the base colors are already linear) from the
   atlas, upload every mip/layer, add a repeat+trilinear sampler and a `material_layout` bind group.
   Thread the new layout through `build_pipeline` and `swap_shader`; bind it as group 4 in `draw`.
   `TerrainRenderer::new` builds the atlas itself. In practice `app.rs` needed a
   one-line change to pass `&gpu.queue` to `TerrainRenderer::new`; the `touched-files`
   list in the front-matter should include `app.rs`.
3. **`terrain.wgsl` — sample + light.** Pass interpolated `tile_uv = corner_uv * vec2(w, h)` (per-block
   tiling) and the flat `layer`. In the fragment: sample albedo+roughness and the tangent-space normal,
   reconstruct the world normal from `FACE_U/FACE_V/FACE_NORMAL`, and use it for diffuse `NdotL`, the
   g-buffer normal, and a Blinn-Phong specular highlight whose strength/shininess come from roughness
   (subtle on matte terrain). Shadow bias keeps using the geometric face normal (avoids acne).

**Validation:** `cargo test` covers the pure atlas (determinism, dimensions, mip count, value ranges,
distinct blocks differ, normal z-positive). The render is validated by running the release app and
capturing a screenshot (terrain textured + relief, not flat), plus the F3 per-pass `terrain` timer to
confirm the cost delta is small. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`. No new render pass and no pass reorder — all within the existing terrain pass (TBDR rule).

**Environment:** all `cargo` from `git-craft/`; `--release` only. Branch `feat/m6c-textures`; PR to `main`.

---

## Stage A — procedural atlas (pure, TDD)

- [x] **A1 `material.rs` generation:** `MaterialAtlas { size, layers, mip_levels, albedo: Vec<Vec<u8>>,
  normal: Vec<Vec<u8>> }` (each `Vec<u8>` is one mip level, layer-major, RGBA8). `build_atlas(size)`:
  per block id, albedo RGB = `block.color()` × deterministic value-noise detail (±~12%), A = a per-block
  roughness; normal = tangent-space normal from a per-block height field (central differences, per-block
  bump scale). Pure hash noise (no RNG state). Tests: deterministic (same bytes twice); `size×size×layers×4`
  per mip; `mip_levels == log2(size)+1`; alpha (roughness) and RGB within range; two different blocks'
  layer-0 albedo differ; normal B channel ≥ 128 (z-positive) at mip 0.
- [x] **A2 mip chain:** box-filter `mip(k) → mip(k+1)` for both arrays down to 1×1. Test: a flat (uniform)
  layer stays uniform across all mips; mip dimensions halve each level.

## Stage B — GPU material binding

- [x] **B1 `terrain.rs` textures:** create two `Rgba8Unorm` `D2Array` textures (`MAX(size)` mips) from the
  atlas, `write_texture` every mip × layer, a repeat + linear/linear/linear (trilinear) sampler, a
  `material_layout` (two textures + sampler, fragment-visible) and its bind group. `new` calls
  `material::build_atlas`. Store the layout + bind group on `TerrainRenderer`.
- [x] **B2 pipeline wiring:** add `material_layout` to `build_pipeline`'s pipeline layout (group 4) and to
  `swap_shader` (hot-reload rebuilds with the same layout); bind group 4 in `draw`. Boots cleanly.

## Stage C — shader

- [x] **C1 `terrain.wgsl` group 4 + vs:** declare `@group(4)` albedo array + normal array + sampler. In
  `vs_main`, output `tile_uv = uv * vec2(w, h)` (interpolated) and `layer = min(tex, 12u)` (flat); drop the
  PALETTE lookup.
- [x] **C2 `terrain.wgsl` fs:** sample `albedo`+`roughness` and the tangent-space `normal`; reconstruct the
  world normal `n = normalize(U*nx + V*ny + N*nz)`; use `n` for `NdotL`, the g-buffer normal, and a
  Blinn-Phong specular term (`shininess = mix(4,64,1-rough)`, `strength = (1-rough)*k`) gated by the same
  shadow/skylight guard; keep shadow bias on the geometric face normal. Keep the PALETTE const out.

## Stage D — validation + docs

- [x] **D1 run + screenshot:** `cargo run --release`, confirm terrain shows texture detail + relief (not flat)
  and the `terrain` F3 timer delta is small; capture a screenshot. Note the feature in CHANGELOG `[Unreleased]`
  and AGENTS.md. Commit: `feat: procedural per-block material textures with normal + roughness (m6)`.

---

## Self-Review
- Spec §10 M6 "texture polish (normal/roughness maps)" → per-block procedural albedo + tangent-space normal
  map + roughness, sampled in the terrain pass and driving normal-mapped diffuse + roughness specular. ✓
- No-proprietary-assets guardrail → every texel is generated in code from a deterministic hash, seeded by
  the block's own base color; nothing is sourced externally. ✓
- TBDR discipline → no new pass, no reorder; sampling + lighting stay inside the existing forward terrain
  pass. Mips are generated on the CPU (no blit pass). The g-buffer normal becomes the perturbed normal so
  GTAO/TAA stay consistent. ✓
- Engine-core discipline → atlas generation is pure and unit-tested first; `terrain.rs` holds the wgpu glue;
  the visual result is validated by a screenshot + the per-pass timer, not by feel.
- Faithfulness → albedo is the existing `block.color()` modulated by gentle noise, so the world keeps its
  identity; the change adds texture, not a re-theme.

