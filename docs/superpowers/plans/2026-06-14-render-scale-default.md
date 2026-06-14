---
title: Render-scale default 0.75× with TAA upsampling
date: 2026-06-14
domain: rendering
type: feat
priority: high
breaking: false
db-migration: false
rls-affecting: false
optimization-required: false
security-required: false
slice: null
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files:
  - git-craft/src/app.rs
  - git-craft/CHANGELOG.md
shared-modules-touched:
  - git-craft/src/render/targets.rs
  - git-craft/src/render/taa.rs
  - git-craft/src/render/post.rs
trigger-tasks-touched: []
---

# Render-Scale Default 0.75× Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Change the engine default render-scale from 1.0 to 0.75, so all HDR passes run at 75% of the swapchain resolution with TAA upsampling to full window size, closing the M6 GPU-budget gap at 1280×720.

**Architecture:** The render-scale safety valve already exists end-to-end. `render_scale: f32` is a field on `App` (initialized to `1.0` at `app.rs:334`). `render_dims()` (`app.rs:122–128`) converts it to an offscreen pixel size. `RenderTargets` is allocated at that scaled size (`targets.rs`). The post pass (`post.rs`) renders the TAA-resolved texture (at scaled res) into the swapchain view (at full res), which is the upsample step — wgpu bilinear-samples up automatically because the source and destination sizes differ. egui/HUD renders directly onto the swapchain view after the post pass (`egui_layer.rs:63–65` uses `config.width/height`, not render dims), so UI is unaffected by render scale. The `R` key cycles 1.0→0.75→0.5→1.0 (`app.rs:1789–1804`) and rebuilds the offscreen chain. Only one line needs to change for the default: `app.rs:334`.

**Tech Stack:** Rust (edition 2024), wgpu 29, winit 0.30. All cargo commands from `git-craft/`. Always `--release`. Benchmark: `cargo run --release -- --bench` at 1280×720.

**Gate field rationale:**
- `optimization-required: false` — this plan IS the performance work; its own `--bench` measurements serve as the validation gate. A separate optimization audit phase would be redundant.
- `security-required: false` — no auth, network, secrets, webhooks, or RLS surface is touched.

---

## Context

The M6 `--bench` baseline at 1280×720 with render-scale 1.0 is:

- GPU p50 ≈ 13.3 ms / p99 ≈ 18.7 ms → **FAIL** (budget: 8.33 ms / 120 fps)
- CPU p50 ≈ 4.6 ms / p99 ≈ 8.9 ms (informational)

Spec §6 (Anti-aliasing) documents the render-scale option as "default 1.0, fallback 0.75 with TAA upsampling." Spec §11 (Risks) names it as the GPU-budget overrun mitigation. This plan promotes the fallback to the default.

**Acceptance target:** `--bench` at 1280×720, render-scale 0.75 → GPU p99 ≤ 8.33 ms AND GPU p50 ≤ 7.0 ms.

---

## File Map

| File | Change |
|---|---|
| `git-craft/src/app.rs:334` | Change `render_scale: 1.0` to `render_scale: 0.75` |
| `git-craft/CHANGELOG.md` | Add `[Unreleased]` entry documenting the default change and before/after bench numbers |

No changes to `targets.rs`, `taa.rs`, `post.rs`, or any shader — the upsample path is already correct (post pass blits `resolved_view`, which is sized to the render dims, into the swapchain view, which is always full-window; wgpu bilinear-samples up). The TAA uniform `params[0]/[1]` (viewport px) are sourced from `rw`/`rh` computed at frame time (`app.rs:989,994`), so they track the scaled resolution automatically.

---

## TAA Upsample Verification (read before touching code)

The following chain is already in place and correct at any render-scale:

1. `render_dims(gpu.config.width, gpu.config.height, self.render_scale)` → `(rw, rh)` (`app.rs:989`)
2. `RenderTargets::new(device, rw, rh)` → all HDR textures at 0.75× window (`targets.rs:64`)
3. TAA resolve writes to `targets.resolved_view` (also `rw×rh`) (`taa.rs:308`)
4. Post pass blits `resolved_view` → swapchain `view` (full window); the pipeline uses `wgpu::FilterMode::Linear` (`post.rs:63–66`), so the upsample is bilinear with no extra code
5. egui draws to `view` with `ScreenDescriptor { size_in_pixels: [config.width, config.height] }` (`egui_layer.rs:63–65`), so HUD/crosshair/hotbar remain full-resolution and unaffected

The `R` cycle handler at `app.rs:1792` calls `next_render_scale(self.render_scale)` which cycles 1.0→0.75→0.5→1.0 (`app.rs:131–139`). After the default changes to 0.75, the first `R` press gives 0.5, the second gives 1.0, the third returns to 0.75 — spec §8 intent is preserved (the cycle still includes all three tiers; the starting point shifts).

---

## Task 1: Measure the M6 baseline (confirm the numbers)

**Files:**
- Read-only: no edits

This task produces the "before" numbers that go in the CHANGELOG and PR description.

- [ ] **Step 1: Run the bench at current default (render-scale 1.0)**

  From `git-craft/`:

  ```bash
  cargo run --release -- --bench 2>&1 | tail -30
  ```

  Expected output format (numbers from CHANGELOG entry; exact values may vary slightly):

  ```
  === git-craft bench report ===
  frames:  600
  ...
  GPU  min ... / p50  13.3 ms / p95 ... / p99  18.7 ms / max ...   FAIL (budget 8.33 ms)
  ```

  Record the actual GPU p50 and p99 values. They become the "before" line in the CHANGELOG.

---

## Task 2: Change the render-scale default to 0.75

**Files:**
- Modify: `git-craft/src/app.rs:334`

- [ ] **Step 1: Locate the initialization site**

  Open `git-craft/src/app.rs`. Search for `render_scale: 1.0` — it appears exactly once, at line 334, inside the `App::new` constructor's struct literal.

  Current code (line 334):
  ```rust
  render_scale: 1.0,
  ```

- [ ] **Step 2: Change the default**

  Replace that one line:
  ```rust
  render_scale: 0.75,
  ```

  Nothing else changes. The rest of the render-scale plumbing (`render_dims`, `rebuild_offscreen`, `next_render_scale`, the `R` handler, the TAA uniform, the egui ScreenDescriptor) is all keyed off `self.render_scale` at runtime and needs no edits.

- [ ] **Step 3: Build to confirm it compiles**

  From `git-craft/`:
  ```bash
  cargo build --release 2>&1 | tail -5
  ```
  Expected: `Finished release profile ... in N.Ns`  (zero errors, zero warnings).

---

## Task 3: Run CI gates

**Files:**
- No additional edits

- [ ] **Step 1: Format**

  From `git-craft/`:
  ```bash
  cargo fmt
  ```
  Expected: no output (file already formatted; the one-line change is a number literal, not a formatting issue).

- [ ] **Step 2: Clippy**

  From `git-craft/`:
  ```bash
  cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
  ```
  Expected: `warning: ... generated N warnings` (none from our change) — or clean. Any warning from our change is a bug; fix before continuing.

- [ ] **Step 3: Tests**

  From `git-craft/`:
  ```bash
  cargo test 2>&1 | tail -10
  ```
  Expected: all tests pass. Our change is a numeric literal in a constructor; it touches no engine-core logic and no unit-tested path.

---

## Task 4: Measure render-scale 0.75 performance

- [ ] **Step 1: Run the bench at the new default (render-scale 0.75)**

  From `git-craft/`:
  ```bash
  cargo run --release -- --bench 2>&1 | tail -30
  ```

  Record GPU p50 and p99.

- [ ] **Step 2: Evaluate against the acceptance target**

  - GPU p99 ≤ 8.33 ms AND GPU p50 ≤ 7.0 ms → **PASS** — proceed to Task 5 (CHANGELOG + commit).
  - One or both metrics above target → **proceed to Task 4C** (conditional single-pass tuning).

- [ ] **Step 3 (conditional — only if FAIL): Identify the highest-cost pass**

  Run in play mode with the F3 HUD visible (press H to toggle if F3 is intercepted by macOS):
  ```bash
  cargo run --release
  ```
  Look at the per-pass GPU times in the Debug HUD (`app.rs:1471–1473`). The HUD lists every timed pass with its ms cost. Identify the single highest-cost pass.

  Common suspects at 0.75× (based on M6 budget estimate from spec §6):
  - Shadow cascades (pass 2): ~1.2 ms at 1.0×; cascades have their own shadow-map resolution, not render-scale — if this is highest, reduce the far cascade update frequency from every 2 frames to every 4 frames.
  - Opaque forward (pass 3): ~2.5–3.0 ms at 1.0×; at 0.75× this should drop to ~1.4–1.7 ms.
  - Volumetric scatter/integrate (pass 4): froxel resolution is fixed; if this is highest, reduce `VOL_D` (froxel depth slices) by 25% in `app.rs`.

  **Apply exactly one tuning** — the minimum change that moves the highest-cost pass below the pro-rata share of the 8.33 ms budget. Document which pass, what changed, and the before/after HUD numbers in a comment in the code and in the CHANGELOG.

  After tuning, re-run the bench (repeat Step 1 and Step 2 of this task). If still FAIL, escalate — do not add more tunings; this is out of scope for this slice.

---

## Task 5: Update CHANGELOG and commit

**Files:**
- Modify: `git-craft/CHANGELOG.md`

- [ ] **Step 1: Add [Unreleased] entry**

  Open `git-craft/CHANGELOG.md`. The `## [Unreleased]` section is currently empty (line 11–12). Add the following block immediately after the `## [Unreleased]` heading, substituting real measured numbers for the placeholders:

  ```markdown
  ### Changed

  - Default render scale changed from 1.0 to 0.75 (spec §6 safety valve): internal HDR
    passes now render at 75% of the window and TAA upsamples to full swapchain resolution.
    UI/HUD (egui) renders directly at full swapchain resolution and is unaffected.
    `--bench` at 1280×720: GPU p50 BEFORE → AFTER ms / p99 BEFORE → AFTER ms
    (target: p99 ≤ 8.33 ms, p50 ≤ 7.0 ms). Verdict: PASS.
    The `R` key still cycles through all three render-scale tiers (0.75 → 0.5 → 1.0 → 0.75).
  ```

  Replace the `BEFORE` and `AFTER` values with the numbers from Tasks 1 and 4. If Task 4C conditional tuning was applied, add a second bullet describing the pass tuning and its measured impact.

- [ ] **Step 2: Commit**

  From the repo root (the worktree):
  ```bash
  git add git-craft/src/app.rs git-craft/CHANGELOG.md
  git commit -m "feat: default render-scale 0.75 to close M6 GPU-budget gap

  Promotes the spec §6 safety valve from fallback to default. Internal HDR
  passes now run at 0.75× the swapchain resolution; TAA upsamples to full
  window. UI/HUD (egui) is unaffected — it renders to the swapchain view
  directly. The R-key cycle still covers all three tiers.

  --bench at 1280x720: GPU p50 BEFORE→AFTER ms / p99 BEFORE→AFTER ms.
  Verdict: PASS (budget 8.33 ms).

  Closes #13"
  ```

  Substitute the real numbers and the issue number before committing.

- [ ] **Step 3: Push**

  ```bash
  git push -u origin feat/render-scale-default
  ```

---

## Task 6: Visual smoke-test

Before opening the PR, run the game and verify no regressions.

- [ ] **Step 1: Launch**

  From `git-craft/`:
  ```bash
  cargo run --release
  ```

- [ ] **Step 2: Check the HUD**

  Press H to open the Debug HUD. Confirm the Scale line reads `0.75 → <width>×<height> (R)` where width×height is 75% of the window size (e.g., at a 1280×720 window: `0.75 → 960×540 (R)`).

- [ ] **Step 3: Cycle render-scale with R**

  Press R once: Scale should read `0.50`. Press R again: `1.00`. Press R again: `0.75` (back to default). No crash, no black screen during transition.

- [ ] **Step 4: Check UI at 0.75×**

  With render-scale at 0.75 (default), verify:
  - Crosshair is centered and crisp (not pixel-doubled or blurry relative to 1.0×).
  - Hotbar slots display correctly with no scaling artifacts.
  - Debug HUD text is sharp (egui renders at native swapchain resolution).

- [ ] **Step 5: Check visuals**

  Walk around, look at terrain, water, shadows, and sky. Confirm:
  - No ghosting or smearing worse than at 1.0× (TAA temporal stability should be equivalent).
  - No obviously blurry geometry that suggests the upsample path is broken (bilinear is expected; harsh pixelation would indicate the wrong resolution is being used).
  - Shadows and volumetric fog are visible and not corrupted.

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Covered by |
|---|---|
| Default render-scale 0.75 (§6) | Task 2 |
| TAA upsamples to full window (§6) | Verified in TAA section above; no code change needed |
| `R` cycle works from 0.75 default (§8) | Verified: next_render_scale(0.75) = 0.5 → 1.0 → 0.75 |
| GPU budget measured with --bench (§8, §11) | Tasks 1 + 4 |
| CHANGELOG [Unreleased] entry | Task 5 |
| Conditional single-pass tuning if still over budget (§11) | Task 4C |

**Placeholder scan:** all steps contain exact commands, expected output forms, and real code. No TBD items.

**Type consistency:** only one symbol touched (`render_scale: 0.75` in the struct literal) — no type changes, no new functions.

Closes #13
