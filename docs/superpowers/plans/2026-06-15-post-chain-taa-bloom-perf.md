---
title: Post-processing chain perf — TAA 5-tap + bloom 5-mip
date: 2026-06-15
domain: render
type: refactor
priority: medium
breaking: false
db-migration: false
rls-affecting: false
optimization-required: false
security-required: false
slice: null
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files:
  - git-craft/assets/shaders/taa.wgsl
  - git-craft/src/render/targets.rs
  - git-craft/src/render/bloom.rs
shared-modules-touched:
  - git-craft/src/render/bloom.rs
trigger-tasks-touched: []
---

# Post-Processing Chain Perf — TAA 5-tap + Bloom 5-mip Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce post-processing CPU-side submission cost at native resolution by shrinking the TAA neighborhood from a 9-tap 3×3 box to a 5-tap plus-shaped cross and dropping the bloom chain's smallest mip pass (6 → 5 mips).

**Architecture:** Two independent, orthogonal levers. Lever A is a pure WGSL edit to `taa.wgsl` — no Rust changes, validated by bench + visual check only. Lever B changes the `bloom_mip_count` pure function in `targets.rs`; it is a tested function so the unit test is written first (TDD). Both levers can be implemented and committed independently; the plan sequences them B-first to keep TDD discipline.

**Tech Stack:** Rust (edition 2024, stable toolchain), WGSL shaders (wgpu / hot-reload), `cargo test` for unit tests, `cargo run --release -- --bench` for perf validation.

---

## Context: what the code does today

**TAA neighborhood (taa.wgsl, lines 60–66):**
The history AABB clamp currently samples all 9 pixels in a 3×3 block centered on the current pixel:
```wgsl
for (var dy = -1; dy <= 1; dy++) {
    for (var dx = -1; dx <= 1; dx++) {
        let c = textureLoad(current_tex, px + vec2(dx, dy), 0).rgb;
        lo = min(lo, c);
        hi = max(hi, c);
    }
}
```
At native 3024×1898 that is ~52 M `textureLoad` calls per frame. Replacing the 3×3 box with a plus cross (center + 4 cardinal neighbors; drop the 4 diagonal corners) cuts loads to ~29 M (~44% reduction). The AABB is slightly looser on diagonals, but the existing ghost-detection blend (`ghost` variable, line 70) compensates by leaning on the current sample when history diverges.

**bloom_mip_count (targets.rs, lines 20–28):**
```rust
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
Called with `half.0, half.1` (i.e. half the window dimensions) in `RenderTargets::new`. At native, half-res is 1512×949, yielding 6 mips. The smallest mip (mip 5) is ~47×30 px — near-zero visual contribution. Raising the threshold from 16 → 32 drops the smallest mip: the loop stops one step earlier, giving 5 mips and saving 2 render passes (1 down + 1 up).

---

## Baseline

Before starting, record the v0.6.0 bench numbers for comparison:

| metric | v0.6.0 baseline |
|--------|----------------|
| CPU p50 (native) | 10.19 ms |
| CPU p99 (native) | 15.27 ms |

---

## Validation

The user runs the bench before and after this PR:

```bash
# from git-craft/
cargo run --release -- --bench
```

**Acceptance criteria:**
1. CPU p99 at native (3024×1898) is measurably lower than 15.27 ms baseline.
2. CPU p50 at native is at or below 10.19 ms.
3. **TAA visual check:** fly fast with diagonal camera motion over foliage and water edges; confirm no new ghosting trails or shimmer vs. the baseline build.
4. **Bloom visual check:** confirm bright light sources (sun, torches, exposed sky) still show a glow halo; confirm no abrupt bloom cutoff artifact vs. the baseline build.

---

## Task 1: Bloom mip-count — TDD (pure Rust, testable)

**Files:**
- Modify: `git-craft/src/render/targets.rs` (function `bloom_mip_count`, lines 20–28; test block lines 266–274)

### Background

`bloom_mip_count` is a pure function with no I/O. The project requires writing the test first for changes to engine-core pure functions.

The current threshold is `>= 16`. Raising it to `>= 32` means the loop exits one iteration sooner at any resolution whose half-size smallest dimension is between 16 and 31 pixels — which includes native (1512×949) and 720p (640×360 half-res).

At 8×8 half-res the loop never runs past n=1 (8 < 32), so minimum stays 1. At very small windows where half-res is < 32 the count will be 1 regardless; this is correct and safe — bloom with a single mip is the same as no bloom chain (no up-passes), which is fine.

- [ ] **Step 1: Write the failing test**

Open `git-craft/src/render/targets.rs` and add the new test `bloom_mip_count_five_mips_at_new_threshold` inside the existing `#[cfg(test)] mod tests` block (after line 274, before the closing `}`):

```rust
#[test]
fn bloom_mip_count_five_mips_at_new_threshold() {
    // After raising the stop threshold to 32 px, native half-res (1512×949)
    // and 720p half-res (640×360) must both yield 5, not 6.
    assert_eq!(
        bloom_mip_count(1512, 949),
        5,
        "native half-res: expect 5 mips with 32px threshold"
    );
    assert_eq!(
        bloom_mip_count(640, 360),
        5,
        "720p half-res: expect 5 mips with 32px threshold"
    );
    // Very small windows: threshold ensures we still get at least 1 mip.
    assert_eq!(
        bloom_mip_count(16, 16),
        1,
        "16×16 half-res: threshold 32 means no iterations, stays at 1"
    );
    assert_eq!(
        bloom_mip_count(32, 32),
        2,
        "32×32 half-res: exactly at threshold, one iteration fires"
    );
}
```

- [ ] **Step 2: Run the test — confirm it fails**

```bash
cd /path/to/repo/git-craft
cargo test bloom_mip_count_five_mips_at_new_threshold -- --nocapture
```

Expected: `FAILED` — the current threshold (16) yields 6 mips for 1512×949 and 640×360, so both `assert_eq!` calls fire.

- [ ] **Step 3: Update `bloom_mip_count` to use the 32-pixel threshold**

In `git-craft/src/render/targets.rs`, replace the function body (lines 20–28):

```rust
/// Mip count for the half-res bloom chain: down to ~32 px, capped at 6.
pub fn bloom_mip_count(w: u32, h: u32) -> u32 {
    let (mut w, mut h, mut n) = (w, h, 1);
    while n < 6 && w >= 32 && h >= 32 {
        w /= 2;
        h /= 2;
        n += 1;
    }
    n
}
```

The only change is `16` → `32` in the while condition (two places) and the doc-comment.

- [ ] **Step 4: Run all tests — confirm they pass**

```bash
cd /path/to/repo/git-craft
cargo test
```

Expected: all tests green, including `bloom_mip_count_five_mips_at_new_threshold` and the pre-existing `bloom_mip_count_scales_with_resolution`. The pre-existing test checks `bloom_mip_count(1512, 982)` — after this change that resolves to 5 (not 6), so that test's assertion will also need updating.

> **Note:** The pre-existing test `bloom_mip_count_scales_with_resolution` asserts `bloom_mip_count(1512, 982) == 6`. That assertion becomes wrong after this change. Update it in the same edit to assert `5`:

```rust
#[test]
fn bloom_mip_count_scales_with_resolution() {
    assert_eq!(
        bloom_mip_count(1512, 982),
        5,
        "native half-res gets 5 mips with 32px threshold"
    );
    assert_eq!(bloom_mip_count(20, 20), 1, "20×20: below 32px threshold, stays at 1");
    assert_eq!(bloom_mip_count(8, 8), 1, "never zero mips");
}
```

Update this test in the same edit as Step 3.

- [ ] **Step 5: Run clippy and fmt**

```bash
cd /path/to/repo/git-craft
cargo fmt
cargo clippy --all-targets -- -D warnings
```

Expected: clean (no warnings, no fmt diffs).

- [ ] **Step 6: Add CHANGELOG entry**

In `CHANGELOG.md` at the repo root, add under `[Unreleased]` (create the section if absent):

```markdown
## [Unreleased]
### Changed
- Bloom mip chain reduced from 6 to 5 mips at native resolution (smallest mip ≥32 px threshold); saves 2 render passes with near-zero visual impact.
```

- [ ] **Step 7: Commit**

```bash
cd /path/to/repo
git add git-craft/src/render/targets.rs CHANGELOG.md
git commit -m "refactor: bloom mip chain 6→5 by raising min-size threshold 16→32 px

Saves 2 render passes (1 down + 1 up) at native and 720p. Smallest
dropped mip was ~47×30 px — negligible visual contribution. Unit test
updated (TDD: test written first).

Refs #13"
git push
```

---

## Task 2: TAA neighborhood — 9-tap → 5-tap plus cross (WGSL only, bench+visual validated)

**Files:**
- Modify: `git-craft/assets/shaders/taa.wgsl` (lines 60–66, the nested loop)

### Background

This is a WGSL shader change. WGSL cannot be unit-tested with `cargo test`; validation is exclusively via bench (CPU p99 before/after) and a visual check for TAA ghosting. There is no Rust code to change.

The current 3×3 nested loop samples pixels at offsets:
```
(-1,-1) ( 0,-1) ( 1,-1)
(-1, 0) ( 0, 0) ( 1, 0)
(-1, 1) ( 0, 1) ( 1, 1)
```
The plus-shaped 5-tap keeps the center and the 4 cardinal neighbors, dropping the 4 diagonals:
```
          ( 0,-1)
(-1, 0)  ( 0, 0)  ( 1, 0)
          ( 0, 1)
```

The resulting AABB is slightly looser on diagonals (it misses the diagonal pixel extremes), but the existing ghost-detection blend (`ghost` variable, computed on line 70) already handles cases where `clamped` diverges far from `history` by increasing the blend toward the current sample. No change to the blend logic is needed.

- [ ] **Step 1: Replace the 3×3 nested loop with a 5-tap plus cross**

In `git-craft/assets/shaders/taa.wgsl`, replace lines 58–66 (the comment + nested loop):

**Before:**
```wgsl
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
```

**After:**
```wgsl
    // Neighborhood AABB clamp (kills ghosting): bound history to the colour
    // box of a 5-tap plus-shaped cross (center + 4 cardinal neighbors).
    // Drops the 4 diagonal corners vs. the old 3×3 box (~44% fewer loads).
    // The AABB is slightly looser on diagonals; the ghost-blend below
    // compensates by leaning on current when history diverges.
    var lo = current;
    var hi = current;
    let neighbors = array<vec2<i32>, 4>(
        vec2( 0, -1),
        vec2(-1,  0),
        vec2( 1,  0),
        vec2( 0,  1),
    );
    for (var i = 0; i < 4; i++) {
        let c = textureLoad(current_tex, px + neighbors[i], 0).rgb;
        lo = min(lo, c);
        hi = max(hi, c);
    }
```

- [ ] **Step 2: Hot-reload and verify shader compiles**

With the game running in another terminal (`cargo run --release`), save the file. The hot-reload system (see `src/render/hot_reload.rs`) will pick up the change within one second. Confirm the console shows no WGSL compile error. If compilation fails, fix the WGSL syntax before proceeding.

Alternatively, build without running:
```bash
cd /path/to/repo/git-craft
cargo build --release
```
Expected: compiles clean. Any WGSL syntax error surfaces as a runtime panic, not a compile error — so do hot-reload or run the binary to confirm.

- [ ] **Step 3: Visual check**

Run the game and perform the visual validation:

```bash
cd /path/to/repo/git-craft
cargo run --release
```

1. Enable the F3 HUD (default key: F3) to monitor per-pass GPU timestamps.
2. Move the camera diagonally at high speed over foliage canopy, water surfaces, and block edges with high contrast (bright sky against dark shadow).
3. Confirm no new ghosting trails, color smearing, or shimmer compared to the v0.6.0 baseline behavior.
4. Confirm the TAA pass GPU time in the HUD is the same or lower than before.

- [ ] **Step 4: Run clippy and fmt (no Rust changes, but confirm CI gates pass)**

```bash
cd /path/to/repo/git-craft
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all green (this task touched only WGSL, so tests and lint are unchanged from Task 1's clean state).

- [ ] **Step 5: Add CHANGELOG entry**

Append to the `[Unreleased]` section in `CHANGELOG.md`:

```markdown
- TAA neighborhood AABB reduced from 9-tap 3×3 box to 5-tap plus cross; ~44% fewer `textureLoad` calls at full native res (~52M → ~29M loads/frame).
```

- [ ] **Step 6: Commit**

```bash
cd /path/to/repo
git add git-craft/assets/shaders/taa.wgsl CHANGELOG.md
git commit -m "refactor: TAA neighborhood 9-tap 3×3 → 5-tap plus cross

Drops 4 diagonal corner samples from the history AABB clamp,
reducing textureLoad calls ~44% at native res. Ghost-blend already
compensates for the looser diagonal AABB. Bench+visual validated.

Refs #13"
git push
```

---

## Task 3: Bench and sign off

- [ ] **Step 1: Run the bench at native resolution**

```bash
cd /path/to/repo/git-craft
cargo run --release -- --bench
```

Record the output CPU p50 and p99 values. Compare to baseline:

| metric | v0.6.0 baseline | after this PR |
|--------|----------------|---------------|
| CPU p50 (native) | 10.19 ms | _measure_ |
| CPU p99 (native) | 15.27 ms | _measure_ |

Acceptance: p99 is lower than 15.27 ms AND p50 is at or below 10.19 ms.

> **Reminder:** The bench window must be the frontmost application on screen when running — GPU timestamps read zero in a background window. See the GPU bench gotcha in memory.

- [ ] **Step 2: Final CI validation**

```bash
cd /path/to/repo/git-craft
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all green.

- [ ] **Step 3: Open PR**

```bash
gh pr create \
  --title "refactor: TAA 5-tap + bloom 5-mip post-chain perf (#13)" \
  --body "$(cat <<'EOF'
## Summary
- TAA neighborhood AABB: 9-tap 3×3 box → 5-tap plus cross (~44% fewer `textureLoad` calls at native res).
- Bloom mip chain: 6 → 5 mips (raise min-size threshold 16 → 32 px, saves 2 render passes).

## Perf results (native 3024×1898)
<!-- fill in after running --bench -->
| metric | v0.6.0 baseline | this PR |
|--------|----------------|---------|
| CPU p50 | 10.19 ms | _TBD_ |
| CPU p99 | 15.27 ms | _TBD_ |

## Visual check
- [ ] Fast diagonal camera motion over foliage/water — no new TAA ghosting
- [ ] Bright light sources (sun, torches) retain bloom glow halo
- [ ] No bloom cutoff artifacts

Refs #13
EOF
)"
```

---

## Self-review

**Spec coverage:**
- Lever A (TAA 5-tap): covered by Task 2.
- Lever B (bloom 5-mip): covered by Task 1.
- Validation section: covered in the preamble and Task 3.
- TDD for the bloom pure function: Task 1 writes the test before the implementation.
- TAA noted as bench+visual-only (no unit test): documented in Task 2 background.
- `Refs #13` trailer on both commits: present.
- CHANGELOG entries for both behavior-affecting changes: present.

**No placeholders:** all code blocks show exact before/after text, exact commands, exact expected outputs.

**Type consistency:** `bloom_mip_count` is the only function name used throughout; no drift.
