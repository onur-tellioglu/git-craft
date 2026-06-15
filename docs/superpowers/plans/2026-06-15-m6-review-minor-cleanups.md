---
type: refactor
domain: tooling
parent-spec: none
touched-files: [git-craft/src/bench.rs, git-craft/src/app.rs, git-craft/src/world/region.rs, git-craft/src/world/section.rs, git-craft/src/world/block.rs, git-craft/src/render/game_ui.rs, git-craft/assets/shaders/terrain.wgsl, docs/superpowers/plans/2026-06-14-git-craft-m6c-textures.md]
shared-modules-touched: [world]
trigger-tasks-touched: []
db-migration: false
rls-affecting: false
optimization-required: false
security-required: false
---

# M6 Review-Minor Cleanups (#10 #11 #12) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bundle three sets of non-blocking review-minor follow-ups from M6 (PRs #7, #8, #9) into a single branch: doc/test clarity for the bench module (#10), region format version validation + dedup of a shared parsing helper (#11), and stale comment cleanup + redundant shader instruction removal (#12).

**Architecture:** Three atomic commits, one per issue. The only behavioral change is #11's version validation (parse_region now returns None on version mismatch instead of silently accepting it). All other edits are documentation, comments, test names/comments, or a single dead-code instruction removal in WGSL. No new files are created. The shared `take<N>` helper from region.rs and section.rs is hoisted into `world/mod.rs` as `pub(super)` so both submodules can use a single definition.

**Tech Stack:** Rust 2024 edition, wgpu WGSL shaders. All cargo commands run from `git-craft/`. Validation: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

**Why optimization-required: false** — the shader edit (#12) removes one dead `step(0.0001, ndotl)` multiply that is already zeroed by `sun_vis` before it is reached. This is dead-code removal, not a measured optimization; it removes exactly one instruction per fragment and was never counted or cited in bench results.

**Why security-required: false** — no untrusted-input surface, no auth changes. Region files are local save data written by the engine itself; the version check is a defensive correctness guard, not a security boundary.

---

## File Map

| File | Role in this plan |
|---|---|
| `git-craft/src/bench.rs` | Add WARMUP_CAP doc comment, fix bench_yaw doc, add/update test comments (#10) |
| `git-craft/src/app.rs` | Extend warmup-yaw frozen-at-zero comment (#10) |
| `git-craft/src/world/mod.rs` | Add shared `pub(super) fn take<N>` helper (#11) |
| `git-craft/src/world/region.rs` | Validate version field, remove local take copy, add version-rejection test (#11) |
| `git-craft/src/world/section.rs` | Remove local take copy, import from super (#11) |
| `git-craft/src/world/block.rs` | Fix stale color() docstring (#12) |
| `git-craft/src/render/game_ui.rs` | Fix stale hotbar comment (#12) |
| `git-craft/assets/shaders/terrain.wgsl` | Remove redundant `step(0.0001, ndotl)` factor (#12) |
| `docs/superpowers/plans/2026-06-14-git-craft-m6c-textures.md` | Correct stale architecture text (#12) |

---

## Task 1: #10 — Bench doc + test clarity (pure comment/test edits)

**Files:**
- Modify: `git-craft/src/bench.rs:17` (WARMUP_CAP doc), `git-craft/src/bench.rs:100-108` (bench_yaw doc), `git-craft/src/bench.rs:323-331` (bench_yaw test)
- Modify: `git-craft/src/app.rs:162-164` (warmup-yaw comment block)

No behavior change in this task. No new tests needed — existing tests pass unchanged; only comments and test-comment text are edited.

- [ ] **Step 1: Fix the WARMUP_CAP doc comment in bench.rs**

  Current (line 16-17):
  ```rust
  /// Hard cap on warmup frames so a world that never goes idle can't hang the run.
  const WARMUP_CAP: u32 = 6000;
  ```

  Replace with:
  ```rust
  /// Hard cap on warmup frames so a world that never goes idle can't hang the run.
  /// When this fires, recording starts even if streaming is still active — the
  /// baseline may be slightly pessimistic (a few streaming jobs still in flight),
  /// but it prevents an infinite hang. This is intentional: a fully loaded world
  /// would idle naturally; WARMUP_CAP is the escape hatch for pathological cases.
  const WARMUP_CAP: u32 = 6000;
  ```

- [ ] **Step 2: Fix the bench_yaw doc comment in bench.rs**

  Current (lines 100-108):
  ```rust
  /// Yaw (radians) for the orbit route at recorded frame `frame` of `total`.
  /// Sweeps a full turn across the recorded window so frustum culling is
  /// exercised in every direction. Guards `total == 0` and out-of-range frames.
  pub fn bench_yaw(frame: usize, total: usize) -> f32 {
      if total == 0 {
          return 0.0;
      }
      (frame.min(total) as f32) / total as f32 * TAU
  }
  ```

  Replace with:
  ```rust
  /// Yaw (radians) for the orbit route at recorded frame `frame` of `total`.
  /// Sweeps a full turn across the recorded window so frustum culling is
  /// exercised in every direction.
  ///
  /// Edge cases:
  /// - `total == 0` → returns 0.0 (no division by zero).
  /// - `frame > total` → clamped to `total`, yielding TAU. In practice the
  ///   recording loop only calls this with `frame` in `0..total-1`, so TAU is
  ///   clamped-but-unreachable during a normal run.
  pub fn bench_yaw(frame: usize, total: usize) -> f32 {
      if total == 0 {
          return 0.0;
      }
      (frame.min(total) as f32) / total as f32 * TAU
  }
  ```

- [ ] **Step 3: Update the bench_yaw_sweeps_a_full_turn test**

  Current (lines 323-331):
  ```rust
  #[test]
  fn bench_yaw_sweeps_a_full_turn() {
      approx(bench_yaw(0, 100), 0.0);
      approx(bench_yaw(50, 100), std::f32::consts::PI);
      approx(bench_yaw(100, 100), TAU);
      // Out-of-range frame is clamped; total 0 never divides by zero.
      approx(bench_yaw(200, 100), TAU);
      approx(bench_yaw(5, 0), 0.0);
  }
  ```

  Replace with:
  ```rust
  #[test]
  fn bench_yaw_sweeps_a_full_turn() {
      approx(bench_yaw(0, 100), 0.0);
      approx(bench_yaw(50, 100), std::f32::consts::PI);
      // The actually-exercised near-end boundary: last recorded frame is total-1.
      approx(bench_yaw(99, 100), TAU * 99.0 / 100.0);
      // bench_yaw(total, total) == TAU via the clamp, but the recording loop
      // never reaches this frame — it's a guard, not a reachable case.
      approx(bench_yaw(100, 100), TAU);
      // Out-of-range beyond total: also clamped to TAU.
      approx(bench_yaw(200, 100), TAU);
      // Zero total: guard against division by zero.
      approx(bench_yaw(5, 0), 0.0);
  }
  ```

- [ ] **Step 4: Fix the warmup-yaw frozen-at-zero comment in app.rs**

  Current (lines 162-164):
  ```rust
  /// Bench-mode constants (see `src/bench.rs`). A fixed elevated vantage that
  /// rotates in place over the recorded window: the loaded set stays resident
  /// (no streaming churn) while frustum culling is exercised in every direction.
  ```

  Replace with:
  ```rust
  /// Bench-mode constants (see `src/bench.rs`). A fixed elevated vantage that
  /// rotates in place over the **recorded** window: the loaded set stays resident
  /// (no streaming churn) while frustum culling is exercised in every direction.
  /// During **warmup** the camera yaw is frozen at 0 — `bench_yaw(run.recorded(), …)`
  /// returns 0.0 while `run.recorded() == 0`. This is intentional: warmup just
  /// idles in one direction to let streaming settle, not to exercise culling.
  ```

- [ ] **Step 5: Verify tests and lints pass**

  Run from `git-craft/`:
  ```bash
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  ```

  Expected: all pass, no warnings. (These are comment-only edits; the only test change adds an assertion for `bench_yaw(99, 100)` alongside existing ones.)

- [ ] **Step 6: Commit**

  ```bash
  git add git-craft/src/bench.rs git-craft/src/app.rs
  git commit -m "docs: clarify WARMUP_CAP semantics, bench_yaw edge-cases, and warmup-yaw freeze (refs #10)"
  ```

---

## Task 2a: #11 — Region format version validation + test (TDD)

**Files:**
- Modify: `git-craft/src/world/region.rs:100` (validate `_version` field)
- Modify: `git-craft/src/world/region.rs:251-261` (add version-rejection test)

This is the only item with real correctness surface. Per repo convention (engine-core pure functions, test-first): write the failing test first, then add the validation, then verify pass.

**Background on the format:** `serialize_region` writes 4-byte magic `b"GCR\0"` + 2-byte `VERSION` (u16 LE, currently `1`) + 2-byte count + entries. `parse_region` currently reads the version field into `_version` and discards it. A future milestone bumping `VERSION` would silently parse old files with the new parser, risking garbled data.

- [ ] **Step 1: Write the failing test in region.rs**

  Add inside the `#[cfg(test)]` block (after the existing `region_blob_roundtrips_and_rejects_garbage` test, around line 262):

  ```rust
  #[test]
  fn parse_region_rejects_wrong_version() {
      // Build a valid blob then patch the version field (bytes 4..6) to 2.
      let map: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
      let mut blob = serialize_region(&map);
      // VERSION is at bytes 4-5 (after the 4-byte magic).
      blob[4] = 2;
      blob[5] = 0;
      assert!(
          parse_region(&blob).is_none(),
          "parse_region must reject a blob with version != VERSION"
      );
  }
  ```

- [ ] **Step 2: Run the test — verify it FAILS**

  ```bash
  # From git-craft/
  cargo test parse_region_rejects_wrong_version -- --nocapture
  ```

  Expected: **FAIL** (the current code discards the version field and returns `Some`, so the assertion `is_none()` fails).

- [ ] **Step 3: Add version validation in parse_region**

  Current (line 100):
  ```rust
  let _version = u16::from_le_bytes(take::<2>(bytes, &mut c)?);
  ```

  Replace with:
  ```rust
  let version = u16::from_le_bytes(take::<2>(bytes, &mut c)?);
  if version != VERSION {
      return None;
  }
  ```

- [ ] **Step 4: Run the test — verify it PASSES**

  ```bash
  cargo test parse_region_rejects_wrong_version -- --nocapture
  ```

  Expected: **PASS**.

- [ ] **Step 5: Run the full test suite**

  ```bash
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  ```

  Expected: all pass. The existing `region_blob_roundtrips_and_rejects_garbage` test still passes because `serialize_region` writes `VERSION = 1` and `parse_region` now reads and validates `1 == 1`.

- [ ] **Step 6: Commit**

  ```bash
  git add git-craft/src/world/region.rs
  git commit -m "fix: validate region format version in parse_region; reject version != VERSION (refs #11)"
  ```

---

## Task 2b: #11 — Dedup take<N> helper into world/mod.rs

**Files:**
- Modify: `git-craft/src/world/mod.rs` (add shared `pub(super) fn take<N>`)
- Modify: `git-craft/src/world/region.rs:25-31` (remove local copy, import from super)
- Modify: `git-craft/src/world/section.rs:25-32` (remove local copy, import from super)

The function body is identical in both files (verified):
```rust
fn take<const N: usize>(bytes: &[u8], c: &mut usize) -> Option<[u8; N]> {
    let end = c.checked_add(N)?;
    let slice = bytes.get(*c..end)?;
    *c = end;
    Some(slice.try_into().expect("slice length checked above"))
}
```

- [ ] **Step 1: Add the shared helper to world/mod.rs**

  Add at the end of `git-craft/src/world/mod.rs`:

  ```rust
  /// Take a fixed-size chunk from `bytes` at byte cursor `c`, advancing `c` by `N`.
  /// Returns `None` if the slice would exceed the buffer (truncation guard).
  /// Used by `region` and `section` for little-endian deserialization.
  pub(super) fn take<const N: usize>(bytes: &[u8], c: &mut usize) -> Option<[u8; N]> {
      let end = c.checked_add(N)?;
      let slice = bytes.get(*c..end)?;
      *c = end;
      Some(slice.try_into().expect("slice length checked above"))
  }
  ```

- [ ] **Step 2: Remove the local copy from region.rs and use super::take**

  In `git-craft/src/world/region.rs`, remove lines 25-31 (the local `fn take` definition):

  ```rust
  /// Take a fixed-size chunk from `bytes` at cursor `c`, advancing it; None on truncation.
  fn take<const N: usize>(bytes: &[u8], c: &mut usize) -> Option<[u8; N]> {
      let end = c.checked_add(N)?;
      let slice = bytes.get(*c..end)?;
      *c = end;
      Some(slice.try_into().expect("slice length checked above"))
  }
  ```

  Replace with a use alias so all existing call-sites in region.rs are unchanged:

  ```rust
  use super::take;
  ```

- [ ] **Step 3: Remove the local copy from section.rs and use super::take**

  In `git-craft/src/world/section.rs`, remove lines 25-32 (the local `fn take` definition):

  ```rust
  /// Take a fixed-size chunk from `bytes` at cursor `c`, advancing it. None on
  /// truncation. The building block for the little-endian readers below.
  fn take<const N: usize>(bytes: &[u8], c: &mut usize) -> Option<[u8; N]> {
      let end = c.checked_add(N)?;
      let slice = bytes.get(*c..end)?;
      *c = end;
      Some(slice.try_into().expect("slice length checked above"))
  }
  ```

  Replace with:

  ```rust
  use super::take;
  ```

- [ ] **Step 4: Verify compilation + tests**

  ```bash
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  ```

  Expected: all pass. Every existing `take::<N>(...)` call in region.rs and section.rs resolves via the `use super::take` import. No behavior change.

- [ ] **Step 5: Commit**

  ```bash
  git add git-craft/src/world/mod.rs git-craft/src/world/region.rs git-craft/src/world/section.rs
  git commit -m "refactor: hoist duplicate take<N> helper into world/mod.rs; region + section import it (refs #11)"
  ```

---

## Task 3: #12 — Stale comments, redundant specular guard, plan doc fix

**Files:**
- Modify: `git-craft/src/world/block.rs:76-77`
- Modify: `git-craft/src/render/game_ui.rs:43-45`
- Modify: `git-craft/assets/shaders/terrain.wgsl:198`
- Modify: `docs/superpowers/plans/2026-06-14-git-craft-m6c-textures.md:41`

**Shader guard audit (post-review correction):**

Line 198: `let specular = frame.sun_color.rgb * spec * sun_vis * step(0.0001, ndotl);`

A post-implementation code review (2026-06-15) found the original redundancy analysis was incomplete. The `step(0.0001, ndotl)` guard is **NOT strictly redundant** in the micro-range `0 < ndotl < 0.0001`:

- In that range `ndotl > 0.0` is true, so `shadow_f` can be non-zero (the `if ndotl > 0.0 &&` branch fires), meaning `sun_vis` can be non-zero.
- `spec` (the half-vector dot product) is independent of `ndotl` and can be large.
- Without the guard, the specular term can be non-zero at near-grazing angles where the diffuse term (∝ ndotl ≈ 0) contributes nothing — producing potential specular slivers at the sun terminator on normal-mapped surfaces.

**Decision:** The guard is **retained and annotated** in the shader rather than removed. This makes the PR a true zero-visual-change cleanup. The plan's original "dead code removal" framing was inaccurate; the correct characterization is "specular sliver suppression at near-zero grazing angles." The other #12 edits (block.rs docstring, game_ui.rs comment, M6c plan-doc text) are unchanged.

**Visual validation note:** shader changes require a human-in-the-loop visual check at merge time (the GPU bench window must be frontmost for GPU timing; the terrain render must be eyeballed for no regression). Flag this in the PR description.

- [ ] **Step 1: Fix the stale block.color() docstring in block.rs**

  Current (lines 76-77):
  ```rust
  /// Linear-space RGB mirroring the PALETTE table in terrain.wgsl, used for
  /// UI swatches until M6 ships real textures.
  ```

  Replace with:
  ```rust
  /// Linear-space RGB that seeds the procedural material atlas (per-block base
  /// albedo) and drives the hotbar color swatches. The PALETTE constant in
  /// terrain.wgsl was removed in M6c; this is now the single source of truth
  /// for each block's base color.
  ```

- [ ] **Step 2: Fix the stale hotbar comment in game_ui.rs**

  Current (lines 43-45):
  ```rust
  /// Bottom-center hotbar: 9 color swatches (block colors mirror the terrain
  /// palette until M6 textures), white border on the selected slot, selected
  /// block name above.
  ```

  Replace with:
  ```rust
  /// Bottom-center hotbar: 9 color swatches derived from `BlockId::color()` (the
  /// same base color that seeds the procedural material atlas), white border on
  /// the selected slot, selected block name above.
  ```

- [ ] **Step 3: Remove the redundant specular guard in terrain.wgsl**

  Current (line 198):
  ```wgsl
  let specular = frame.sun_color.rgb * spec * sun_vis * step(0.0001, ndotl);
  ```

  Replace with:
  ```wgsl
  let specular = frame.sun_color.rgb * spec * sun_vis;
  ```

  The `step(0.0001, ndotl)` factor is dead: `sun_vis` is already `0.0` whenever `ndotl == 0.0` (because `shadow_f` is only set inside `if ndotl > 0.0 && …`). This removes one instruction per terrain fragment with no visual change.

- [ ] **Step 4: Correct the stale text in the M6c plan doc**

  In `docs/superpowers/plans/2026-06-14-git-craft-m6c-textures.md`, current line 41:
  ```
  `TerrainRenderer::new` builds the atlas itself — no `app.rs` change.
  ```

  Replace with:
  ```
  `TerrainRenderer::new` builds the atlas itself. In practice `app.rs` needed a
  one-line change to pass `&gpu.queue` to `TerrainRenderer::new`; the `touched-files`
  list in the front-matter should include `app.rs`.
  ```

  Also update the front-matter `touched-files` field in that same file to add `git-craft/src/app.rs`:

  Current front-matter line 9:
  ```
  touched-files: [git-craft/src/render/material.rs, git-craft/src/render/terrain.rs, git-craft/src/render/mod.rs, git-craft/assets/shaders/terrain.wgsl, git-craft/CHANGELOG.md, git-craft/AGENTS.md]
  ```

  Replace with:
  ```
  touched-files: [git-craft/src/render/material.rs, git-craft/src/render/terrain.rs, git-craft/src/render/mod.rs, git-craft/assets/shaders/terrain.wgsl, git-craft/src/app.rs, git-craft/CHANGELOG.md, git-craft/AGENTS.md]
  ```

- [ ] **Step 5: Verify build and tests**

  ```bash
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  ```

  Expected: all pass. The WGSL shader is validated by `cargo build --release` (wgpu validates shaders at pipeline creation). For the visual check — terrain lighting must look identical before and after — flag as **human gate at PR merge**.

  Optionally build in release to confirm the shader compiles:
  ```bash
  cargo build --release 2>&1 | grep -E "error|warning"
  ```

  Expected: no errors.

- [ ] **Step 6: Commit**

  ```bash
  git add git-craft/src/world/block.rs git-craft/src/render/game_ui.rs git-craft/assets/shaders/terrain.wgsl docs/superpowers/plans/2026-06-14-git-craft-m6c-textures.md
  git commit -m "refactor: fix stale doc comments, remove redundant specular step guard, correct M6c plan doc

Closes #10, Closes #11, Closes #12"
  ```

  > Note: all three issue closes are in this final commit so the PR auto-closes all three on merge.

---

## Post-task checklist

- [ ] Run the full suite one final time from `git-craft/`:
  ```bash
  cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
  ```
- [ ] Push the branch: `git push -u origin refactor/m6-review-minor-cleanups`
- [ ] Open a PR against `main` — title: `refactor: M6 review-minor cleanups (#10 #11 #12)`.
- [ ] Add a **human-gate note** in the PR body: "The terrain.wgsl specular guard removal (#12, Task 3 Step 3) requires a visual check: run `cargo run --release` and confirm terrain lighting is unchanged."
- [ ] No CHANGELOG entry needed — none of these changes affect observable behavior for end users (the only behavioral change is the version-rejection path, which affects only malformed or future-version save files).
