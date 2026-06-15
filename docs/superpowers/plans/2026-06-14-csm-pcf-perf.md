---
title: "CSM + PCF GPU cost reduction (render-scale 1.0)"
date: 2026-06-14
domain: rendering
type: refactor
priority: high
breaking: false
db-migration: false
rls-affecting: false
optimization-required: false
security-required: false
slice: 1
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files:
  - git-craft/src/render/shadow.rs
  - git-craft/assets/shaders/terrain.wgsl
  - CHANGELOG.md
trigger-tasks-touched: []
shared-modules-touched:
  - git-craft/src/render/shadow.rs
---

# CSM + PCF GPU Cost Reduction — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce the per-frame GPU cost of shadow-cascade depth rasterization and/or PCF shadow evaluation to produce a measured p99 improvement at 1280×720, render-scale 1.0, without introducing shadow acne or aliasing regression.

**Architecture:** The bottleneck hypothesis (issue #13) is that the dominant GPU cost at render-scale 1.0 is resolution-independent of the main framebuffer: 3 × 2048² depth passes (PASS_SHADOW0/1/2 in `src/app.rs`) and a 5×5 = 25-tap PCF kernel in `terrain.wgsl` `shadow_factor()`. The plan profiles first to confirm which passes are dominant, then applies the single highest-impact change the data points to. Two candidate optimizations are described (PCF kernel reduction and/or far-cascade resolution drop); only the one(s) validated by profiling are implemented.

**Tech Stack:** Rust/wgpu/WGSL, `cargo run --release -- --bench`, F3 HUD per-pass GPU timestamps (PASS_SHADOW0=slot 1, PASS_SHADOW1=slot 2, PASS_SHADOW2=slot 3, PASS_MAIN=slot 4).

---

## Shippability contract (read before starting)

This PR is SHIPPABLE when:
- **Hard bar (this PR):** a measured GPU p99 reduction at render-scale 1.0 is recorded (before → after), AND there is no visible shadow acne or aliasing regression in normal play, AND `CHANGELOG.md [Unreleased]` is updated with the before/after numbers.
- **Aspiration (issue #13's full goal, may require multiple PRs):** p99 ≤ 8.33 ms / p50 ≤ 7.0 ms at 1280×720.
- The final commit references `Refs #13` — NOT `Closes #13`. The orchestrator closes #13 only when the full budget AC is met post-merge.

---

## File map

| File | Role in this plan |
|---|---|
| `git-craft/src/render/shadow.rs` | Defines `SHADOW_RESOLUTION` (currently `2048`), `CASCADE_COUNT` (3), per-cascade update cadence, and the GPU depth-pass renderer. Far-cascade resolution change lives here if chosen. |
| `git-craft/assets/shaders/terrain.wgsl` | Contains `shadow_factor()` with the 5×5 PCF loop (lines 118–143) and the `SHADOW_TEXEL` constant (line 65, currently `1.0 / 2048.0`). PCF kernel reduction lives here if chosen. |
| `CHANGELOG.md` | Must receive an `[Unreleased]` entry with before/after GPU numbers. |

---

## Background: what the code actually does

### CSM depth passes (`src/render/shadow.rs`)

- Three cascades, each a 2048×2048 depth-only render pass.
- `SHADOW_RESOLUTION: u32 = 2048` (line 8) applies uniformly to all three cascades — a single constant controls all cascade texture sizes.
- Cascade 0 and 1 run every frame (`cascade_due` lines 99–108). Cascade 2 runs every other frame.
- Each cascade rasterizes up to ~1k sections via `draw_indexed_indirect`. The depth texture is allocated as a single `D2Array` with 3 layers (line 189–203), so resizing only the far cascade requires splitting into separate textures.
- The depth bias is configured in `build_pipeline` (`constant: 3, slope_scale: 3.0` — these resist acne on current geometry and must not be loosened carelessly).
- `ShadowUniform.texels` carries `texel_world` (world size per shadow texel) per cascade. This is used in `terrain.wgsl` for the normal-offset bias: `pos = world_pos + normal * shadow.texels[c] * 3.0`. If cascade 2's resolution is reduced, its `texel_world` increases automatically (computed as `2.0 * radius / resolution` in `fit_light_matrix`), so the normal-offset bias self-scales correctly.

### PCF kernel (`assets/shaders/terrain.wgsl`)

- `SHADOW_TEXEL: f32 = 1.0 / 2048.0` (line 65) — hardcoded to match the 2048² atlas.
- `shadow_factor()` (lines 118–143) loops `dy` from -2 to +2 and `dx` from -2 to +2: 5×5 = 25 `textureSampleCompareLevel` taps per lit opaque fragment, each a hardware 2×2 bilinear compare (wgpu/Metal `comparison_sampler` with linear filtering), so the effective kernel is ~6-texel-radius soft shadow.
- Reducing to 3×3 (loop from -1 to +1, 9 taps, divide by 9) cuts the per-fragment shadow cost by ~64% at the expense of a visibly harder shadow penumbra. Whether that matters at voxel scale is confirmed visually.

### F3 HUD pass slots

Defined in `src/app.rs` lines 22–46:
```
slot 0 = luts
slot 1 = shadow0   ← PASS_SHADOW0
slot 2 = shadow1
slot 3 = shadow2
slot 4 = main      ← PASS_MAIN (opaque terrain + PCF)
slot 5 = gtao
slot 6 = volumetric
slot 7 = composite
slot 8 = water
slot 9 = taa
slot 10 = bloom
slot 11 = exposure
slot 12 = post
```
The F3 HUD displays these by label and value. `GpuTimer::total_ms()` sums all slots and is reported as the bench's "GPU" metric.

### `--bench` mode

`cargo run --release -- --bench` (from `git-craft/`) runs a deterministic fixed-vantage flythrough at the full 384-block render distance (frozen at noon, persistence worker disabled), warms until streaming goes idle, records 600 frames (override with `--bench-frames N`), and prints min/mean/p50/p95/p99/max for CPU frame time and GPU pass time. The GPU metric is `GpuTimer::total_ms()` — sum of all pass slots. PASS/FAIL verdict is against 8.33 ms.

**M6 baseline (from CHANGELOG.md):** GPU p50 ≈ 13.3 ms / p99 ≈ 18.7 ms at 1280×720 render-scale 1.0.

---

## Task 1 — Establish the profiling baseline (no code change)

**Files:** none modified.

**Goal:** Run `--bench`, capture per-pass GPU timestamps from the F3 HUD, and identify which pass(es) dominate the GPU budget. Record these numbers as the baseline that every subsequent task measures against.

- [ ] **Step 1: Build and run the bench**

  From `git-craft/`:
  ```bash
  cargo run --release -- --bench 2>&1 | tee /tmp/bench-baseline.txt
  ```

  The program will open a window, fly the deterministic route, and print a report. Look for output like:
  ```
  GPU  min=X.X  mean=X.X  p50=X.X  p95=X.X  p99=X.X  max=X.X  ms  [FAIL]
  CPU  min=X.X  mean=X.X  p50=X.X  p95=X.X  p99=X.X  max=X.X  ms
  ```

- [ ] **Step 2: Read the F3 HUD per-pass times**

  While the bench is flying (before the report prints), press F3 to open the debug HUD. Note the per-pass GPU milliseconds for each labeled slot. Record the values of:
  - `shadow0`, `shadow1`, `shadow2` (CSM depth passes, slots 1–3)
  - `main` (opaque terrain + PCF, slot 4)
  - `gtao`, `volumetric` (slots 5–6, to rule them in or out)

  The F3 HUD refreshes continuously; read values from the steady-flight portion of the bench (not the warmup stutter).

  > If the HUD is not visible during `--bench` mode, check whether the window is focusable. Alternatively, add a temporary `eprintln!` to `GpuTimer::after_submit` to print `pass_ms` each frame, rebuild with `--release`, and redirect to a file.

- [ ] **Step 3: Determine the dominant pass(es)**

  Compute the fraction each pass contributes to `GpuTimer::total_ms()` (total = bench's GPU number, approximately the sum of all HUD slots).

  Decision gate — pursue ONE of the following:

  | Condition | Action |
  |---|---|
  | shadow0+1+2 > 50% of total AND main > 20% | Implement BOTH PCF reduction (Task 2) AND far-cascade resolution (Task 3, optional) |
  | shadow0+1+2 > 50% of total AND main ≤ 20% | Implement far-cascade resolution reduction only (skip Task 2) |
  | main > 50% of total | Implement PCF reduction only (Task 2); skip Task 3 |
  | gtao or volumetric > 30% of total | Re-evaluate: those passes are outside this plan's scope; document the finding and stop (set STATUS:BLOCKED) |

- [ ] **Step 4: Record the baseline**

  Write the baseline numbers into a comment block at the top of `CHANGELOG.md [Unreleased]`:
  ```markdown
  ## [Unreleased]
  <!-- Baseline (2026-06-14, M6 v0.6.0, 1280×720 render-scale 1.0):
       GPU p50 ≈ 13.3 ms / p99 ≈ 18.7 ms (from CHANGELOG v0.6.0)
       shadow0=Xms shadow1=Xms shadow2=Xms main=Xms (fill in from HUD)
  -->
  ```
  (This comment is removed when the final entry is written in Task 4.)

---

## Task 2 — PCF kernel reduction: 5×5 → 3×3 (25 taps → 9 taps)

**Prerequisite:** Task 1 Step 3 confirms `main` is a significant contributor (≥20% of total GPU time). If the decision gate in Task 1 says skip this task, do NOT implement it.

**Files:**
- Modify: `git-craft/assets/shaders/terrain.wgsl` (lines 115–143 — `shadow_factor` function and `SHADOW_TEXEL` constant)

- [ ] **Step 1: Understand the current kernel**

  Read `shadow_factor()` in `terrain.wgsl` lines 115–143. The current structure:
  ```wgsl
  const SHADOW_TEXEL: f32 = 1.0 / 2048.0;  // line 65

  // inside shadow_factor():
  var sum = 0.0;
  for (var dy = -2; dy <= 2; dy++) {
      for (var dx = -2; dx <= 2; dx++) {
          let o = vec2(f32(dx), f32(dy)) * SHADOW_TEXEL;
          sum += textureSampleCompareLevel(shadow_map, shadow_samp, uv + o, c, p.z);
      }
  }
  return sum / 25.0;
  ```

- [ ] **Step 2: Edit `terrain.wgsl` — reduce loop bounds and divisor**

  Change the loop from `[-2, 2]` (5×5=25 taps) to `[-1, 1]` (3×3=9 taps):

  In `git-craft/assets/shaders/terrain.wgsl`, replace lines 135–142:
  ```wgsl
  var sum = 0.0;
  for (var dy = -2; dy <= 2; dy++) {
      for (var dx = -2; dx <= 2; dx++) {
          let o = vec2(f32(dx), f32(dy)) * SHADOW_TEXEL;
          sum += textureSampleCompareLevel(shadow_map, shadow_samp, uv + o, c, p.z);
      }
  }
  return sum / 25.0;
  ```
  with:
  ```wgsl
  var sum = 0.0;
  for (var dy = -1; dy <= 1; dy++) {
      for (var dx = -1; dx <= 1; dx++) {
          let o = vec2(f32(dx), f32(dy)) * SHADOW_TEXEL;
          sum += textureSampleCompareLevel(shadow_map, shadow_samp, uv + o, c, p.z);
      }
  }
  return sum / 9.0;
  ```

  No other change to `shadow_factor` is needed. The normal-offset bias, cascade selection, and UV-bounds check are all unchanged.

- [ ] **Step 3: Verify the shader compiles**

  ```bash
  cd git-craft && cargo test --release 2>&1 | grep -E "test.*shadow|error\[|FAILED|ok"
  ```

  The test suite includes a headless shader-compilation test. Expected: all tests pass. If there is a WGSL compile error, fix it before proceeding.

- [ ] **Step 4: Visual regression check — play in release mode**

  ```bash
  cd git-craft && cargo run --release
  ```

  Fly around in daylight with the sun at a medium angle. Observe shadow edges on flat ground (grass), vertical walls (stone faces), and angled surfaces (hill slopes). Check for:

  - Shadow **acne** (dark speckles on lit faces, especially on surfaces grazing the sun angle): must be absent. The depth bias (`constant: 3, slope_scale: 3.0`) and normal-offset bias should already prevent this; the kernel reduction does not affect bias.
  - **Aliasing regression**: the 3×3 kernel produces a harder penumbra than 5×5. Some visible hardening is expected and acceptable at voxel scale; jagged pixel-stepping on shadow edges that was not present before is NOT acceptable. If aliasing is obviously worse, increase the loop to `[-1, 2]` / `[2, -1]` alternating (a 3×4 Poisson-style jitter) or stop at this task and document why.
  - Press F3 and verify `main` pass time has dropped compared to the baseline.

- [ ] **Step 5: Commit**

  Run the CI gate first:
  ```bash
  cd git-craft && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
  ```
  All must pass. Then:
  ```bash
  git add git-craft/assets/shaders/terrain.wgsl
  git commit -m "refactor: reduce PCF shadow kernel from 5×5 to 3×3 (25→9 taps)

  The 5x5 PCF kernel in terrain.wgsl shadow_factor() costs 25
  textureSampleCompareLevel taps per lit opaque fragment. Reducing to
  3x3 (9 taps, ~64% fewer) cuts main-pass shadow evaluation cost
  while keeping the hardware 2x2 bilinear compare on each tap, so
  the penumbra gradient is still smooth at voxel texel scale.

  No change to depth bias, normal-offset bias, or cascade selection.
  Visual regression verified: no acne on flat/angled surfaces.

  Refs #13"
  git push
  ```

---

## Task 3 — Far-cascade resolution reduction: cascade 2 from 2048 → 1024 (optional)

**Prerequisite:** Task 1 Step 3 confirms shadow passes contribute ≥50% of total GPU time AND the profiling after Task 2 (or standalone if Task 2 was skipped) still shows PASS_SHADOW2 as a meaningful contributor. If the budget is already met after Task 2, skip this task.

**Context:** Cascade 2 covers the far shadow range (beyond cascades 0 and 1 and up to `SHADOW_FAR = 360.0` blocks away). At 1024², the shadow-map texel footprint on far geometry doubles (from ~0.35 m/texel to ~0.70 m/texel at the edge of cascade 2's range). Because far geometry is small on screen, the texel doubling is rarely visible. The normal-offset bias in `terrain.wgsl` self-scales correctly because `texel_world` is recomputed as `2.0 * radius / resolution` in `fit_light_matrix` — no manual bias adjustment needed.

**Complexity:** The current shadow texture is allocated as a single 2048×2048×3 `D2Array` in `ShadowRenderer::new` (shadow.rs line 189). Reducing only cascade 2 requires either (a) keeping all three layers at 2048² (wasteful, no saving) or (b) splitting cascade 2 into a separate texture with its own `TextureView`, pipeline bind group, and sampling in the shader. Option (b) increases code complexity substantially and is the recommended approach for a real savings. Assess the complexity before committing.

**Alternative (simpler, less saving):** Reduce ALL cascades from 2048→1536 by changing `SHADOW_RESOLUTION` (one constant). This reduces memory and GPU rasterization cost uniformly, keeps the single-array layout, and is safe as long as visual regression testing confirms no acne. Savings are smaller (~44% area reduction per cascade vs. 75% for cascade 2 alone).

**This task implements the simpler uniform reduction to 1536 first.** If 1536 shows visual problems, fall back to evaluating the per-cascade split approach.

**Files:**
- Modify: `git-craft/src/render/shadow.rs` (line 8: `SHADOW_RESOLUTION`)
- Modify: `git-craft/assets/shaders/terrain.wgsl` (line 65: `SHADOW_TEXEL`)

- [ ] **Step 1: Change `SHADOW_RESOLUTION` in `shadow.rs`**

  In `git-craft/src/render/shadow.rs`, change line 8:
  ```rust
  pub const SHADOW_RESOLUTION: u32 = 2048;
  ```
  to:
  ```rust
  pub const SHADOW_RESOLUTION: u32 = 1536;
  ```

  The texture allocation at line 192 reads `width: SHADOW_RESOLUTION, height: SHADOW_RESOLUTION` — no further change needed there. The `fit_light_matrix` call at line 366 passes `SHADOW_RESOLUTION` — self-adjusts. The `ShadowUniform.texels` values are recomputed each frame from `fit.texel_world` — self-adjusts.

- [ ] **Step 2: Update `SHADOW_TEXEL` constant in `terrain.wgsl`**

  The constant `SHADOW_TEXEL: f32 = 1.0 / 2048.0` (terrain.wgsl line 65) defines the per-texel UV step in the PCF loop. It must match the actual shadow map resolution.

  Change line 65:
  ```wgsl
  const SHADOW_TEXEL: f32 = 1.0 / 2048.0;
  ```
  to:
  ```wgsl
  const SHADOW_TEXEL: f32 = 1.0 / 1536.0;
  ```

- [ ] **Step 3: Update the test that checks the uniform layout**

  In `shadow.rs` the test `shadow_uniform_layout_matches_wgsl` at line 570 does NOT depend on resolution — it tests struct offsets. No test change needed.

  However, verify the existing tests still pass:
  ```bash
  cd git-craft && cargo test shadow 2>&1
  ```
  Expected: all shadow tests pass (the math tests use hard-coded resolution `const RES: u32 = SHADOW_RESOLUTION` on line 448 which now points to 1536 — the geometry tests remain valid).

- [ ] **Step 4: Visual regression check**

  ```bash
  cd git-craft && cargo run --release
  ```

  Fly through a scene with long shadow casts (noon, medium sun angle, large flat terrain in view). Check for:

  - **Acne on near surfaces** (cascade 0 coverage, ~0–40 blocks): the near cascade's texel doubles from ~0.09 m to ~0.12 m. The slope-scale bias (`3.0`) and normal-offset (`3.0 × texel_world`) should still prevent acne. If dark stippling appears on lit stone/grass faces, the bias needs tightening (increase `constant` in `DepthBiasState` from `3` to `4` or `5`).
  - **Shadow edge quality on far terrain** (cascade 2 coverage): some softness reduction is expected and acceptable. Hard pixel aliasing that breaks the visual style is NOT acceptable.
  - Press F3 and verify `shadow0`, `shadow1`, `shadow2` pass times have dropped.

- [ ] **Step 5: Commit**

  ```bash
  cd git-craft && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
  ```
  All must pass. Then:
  ```bash
  git add git-craft/src/render/shadow.rs git-craft/assets/shaders/terrain.wgsl
  git commit -m "refactor: reduce shadow-map resolution from 2048 to 1536

  Uniform cascade resolution reduction cuts shadow depth-pass GPU cost
  by ~44% (area scales as resolution^2: 1536^2/2048^2 ≈ 0.56). The
  single SHADOW_RESOLUTION constant controls all three cascade layers;
  the texture D2Array and fit_light_matrix calls adjust automatically.
  SHADOW_TEXEL in terrain.wgsl updated to match 1/1536 so PCF tap
  offsets remain correct. Normal-offset bias self-scales via texel_world.

  Verified: no shadow acne on flat or grazing-angle surfaces. Far shadow
  edge softness is reduced but acceptable at voxel geometry scale.

  Refs #13"
  git push
  ```

---

## Task 4 — Re-measure and update CHANGELOG

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Run `--bench` with all changes applied**

  ```bash
  cd git-craft && cargo run --release -- --bench 2>&1 | tee /tmp/bench-after.txt
  ```

  Record the GPU p50 and p99 from the report output.

- [ ] **Step 2: Capture F3 HUD per-pass times post-optimization**

  Open F3 during the bench flight. Record `shadow0`, `shadow1`, `shadow2`, and `main` values.

- [ ] **Step 3: Write `CHANGELOG.md [Unreleased]` entry**

  Remove the temporary comment block from Task 1 Step 4 and replace with a proper entry:

  ```markdown
  ## [Unreleased]

  ### Performance

  - Reduced PCF shadow kernel in `terrain.wgsl` from 5×5 (25 taps) to 3×3 (9 taps),
    cutting per-fragment shadow evaluation cost by ~64%. Shadow penumbra softness is
    slightly reduced but remains visually acceptable at voxel scale.
  - Reduced shadow-map resolution from 2048² to 1536² across all three cascades,
    cutting cascade depth-pass GPU cost by ~44% (area ratio). Normal-offset and
    slope-scale depth bias self-scale via `texel_world`; no acne introduced.
  - 1280×720 render-scale 1.0 GPU benchmark (before → after):
    p50 ≈ 13.3 ms → X.X ms / p99 ≈ 18.7 ms → X.X ms
    (fill in actual numbers from /tmp/bench-after.txt)
  ```

  Fill in the actual measured numbers before committing.

- [ ] **Step 4: Commit the CHANGELOG**

  ```bash
  git add CHANGELOG.md
  git commit -m "chore: record CSM+PCF perf improvement in CHANGELOG (Refs #13)"
  git push
  ```

---

## Task 5 — Final validation and PR

- [ ] **Step 1: Full CI gate**

  ```bash
  cd git-craft && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
  ```
  All must pass.

- [ ] **Step 2: Confirm shippability**

  Verify all three hard-bar conditions:
  1. GPU p99 post-bench is lower than the 18.7 ms baseline (any measured improvement is sufficient; 8.33 ms is the aspiration, not the hard bar for this PR).
  2. No visible shadow acne or aliasing regression was observed in play (Tasks 2 Step 4 / 3 Step 4).
  3. `CHANGELOG.md [Unreleased]` has the before/after numbers.

- [ ] **Step 3: Open the PR**

  ```bash
  gh pr create \
    --title "refactor: reduce CSM shadow + PCF GPU cost at render-scale 1.0" \
    --body "## Summary

  - PCF kernel in \`terrain.wgsl\` reduced from 5×5 (25 taps) to 3×3 (9 taps)
  - Shadow-map resolution reduced from 2048² to 1536² (uniform across all cascades)
  - GPU p99 at 1280×720 render-scale 1.0: before ≈ 18.7 ms → after X.X ms

  ## Test plan

  - [ ] \`cargo fmt\` + \`clippy\` + \`test\` all pass
  - [ ] \`cargo run --release -- --bench\` shows lower GPU p50/p99 vs. M6 baseline
  - [ ] F3 HUD per-pass times for shadow0/1/2 and main are lower
  - [ ] Visual play-test: no shadow acne, no aliasing regression on lit terrain

  Refs #13

  🤖 Generated with [Claude Code](https://claude.com/claude-code)"
  ```

---

## Self-review

**Spec coverage:**
- Issue #13 AC "F3 HUD per-pass timestamps captured; dominant pass identified" → Task 1.
- Issue #13 AC "targeted optimization applied" → Task 2 (PCF) and/or Task 3 (resolution).
- Issue #13 AC "`--bench` before/after recorded" → Tasks 1 and 4.
- Issue #13 AC "no visual regression" → Tasks 2 Step 4 and 3 Step 4.
- Issue #13 AC "CHANGELOG [Unreleased] updated" → Task 4 Step 3.

**Shippability gate:** explicitly stated above the task list and in Task 5 Step 2. This PR does NOT have to reach 8.33 ms; it only has to show a measured improvement and no regression.

**No placeholders:** all code blocks show exact diffs, exact command lines, and exact expected output patterns.

**Type consistency:** `SHADOW_RESOLUTION` is `u32` throughout; `SHADOW_TEXEL` is `f32`; both touch the same Rust constant and WGSL constant in the same tasks that modify them.

**Apple TBDR constraint:** this plan makes no pass-ordering changes, adds no Z-prepass, and does not touch render-pass `LoadOp`/`StoreOp` configuration. It only reduces loop iteration count in an existing fragment shader and resizes an existing depth texture.
