---
title: git-craft Open-Source Release
date: 2026-06-14
domain: tooling
type: documentation
priority: medium
breaking: false
db-migration: false
rls-affecting: false
slice: null
parent-spec: docs/superpowers/specs/2026-06-14-git-craft-open-source-design.md
touched-files:
  - dabcraft/Cargo.toml
  - dabcraft/src/app.rs
  - dabcraft/src/render/gpu.rs
  - dabcraft/src/mesh/greedy.rs
  - .gitignore
  - CLAUDE.md
  - README.md
  - LICENSE-MIT
  - LICENSE-APACHE
  - CONTRIBUTING.md
  - CODE_OF_CONDUCT.md
  - .github/**
  - docs/superpowers/**
trigger-tasks-touched: []
shared-modules-touched: []
---

# git-craft Open-Source Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the project to `git-craft`, dual-license it MIT/Apache-2.0, add standard contribution infrastructure, and publish it to GitHub as a private repository ready to be flipped public.

**Architecture:** A rename pass (directory, Cargo package, code strings, docs token) followed by additive root/`.github` files (license, README, CONTRIBUTING, CoC, templates, CI), then a git publishing flow (merge latest code to `main`, atomic commits on a `chore/` branch, create the repo, push, open a PR).

**Tech Stack:** Rust (edition 2024, wgpu 29), Git, GitHub CLI (`gh`), GitHub Actions.

---

## Notes for the implementer

- **No remote exists yet** and no GitHub issue is associated. Commit messages therefore carry **no** `refs #N` / `closes #N` trailers.
- Per the project's git conventions: atomic commits, message format `type: what and why`, **no** `Co-Authored-By` trailer, **no** `--no-verify`.
- The crate lives in the `dabcraft/` subdirectory (renamed to `git-craft/` in Task 2). Run `cargo` commands from inside that directory.
- Outward-facing / irreversible steps (creating the GitHub repo) are in the final task — get the maintainer's explicit go-ahead before running them.

---

## Task 1: Pre-flight and branch setup

**Files:** none (git operations only)

- [ ] **Step 1: Confirm a clean working tree**

Run: `git -C /Users/onurtellioglu/Github/Minecraft status --short`
Expected: empty output (the design spec commit is already in history). If unrelated modified files appear, stop and ask the maintainer — do not proceed.

- [ ] **Step 2: Confirm GitHub CLI auth**

Run: `gh auth status`
Expected: shows a logged-in account. If not authenticated, stop and ask the maintainer to run `gh auth login` (suggest typing `! gh auth login` in the session).

- [ ] **Step 3: Merge the latest code into `main`**

Run:
```bash
git -C /Users/onurtellioglu/Github/Minecraft checkout main
git -C /Users/onurtellioglu/Github/Minecraft merge --ff-only feat/m5-shaders
```
Expected: a fast-forward merge (main is strictly behind feat/m5-shaders). If `--ff-only` fails, stop and ask the maintainer how to reconcile — do not force a merge commit unprompted.

- [ ] **Step 4: Create the working branch**

Run:
```bash
git -C /Users/onurtellioglu/Github/Minecraft checkout -b chore/open-source-setup
```
Expected: switched to a new branch `chore/open-source-setup`.

---

## Task 2: Rename dabcraft → git-craft

**Files:**
- Move: `dabcraft/` → `git-craft/`
- Modify: `git-craft/Cargo.toml`, `git-craft/src/app.rs`, `git-craft/src/render/gpu.rs`, `git-craft/src/mesh/greedy.rs`, `.gitignore`, `CLAUDE.md`
- Modify (token only): `docs/superpowers/specs/*.md`, `docs/superpowers/plans/*.md`

> **Preserve list** (do NOT change these): the **filenames** of dated docs in `docs/superpowers/` (e.g. `2026-06-11-dabcraft-design.md`) and any **path reference** that points at them (notably the `Key documents` link in `CLAUDE.md`, which must keep `docs/superpowers/specs/2026-06-11-dabcraft-design.md`). Only the project-name token inside file *bodies* changes.

- [ ] **Step 1: Move the crate directory (preserving history)**

Run: `git -C /Users/onurtellioglu/Github/Minecraft mv dabcraft git-craft`
Expected: the directory and all tracked files move to `git-craft/`.

- [ ] **Step 2: Rename the Cargo package**

Modify `git-craft/Cargo.toml` — change the package name line:
```toml
[package]
name = "git-craft"
```
(Leave `version`, `edition`, and dependencies untouched. Cargo metadata is added in Task 3.)

- [ ] **Step 3: Replace user-facing strings and comments in code**

In `git-craft/src/app.rs`, the window title:
```rust
.create_window(Window::default_attributes().with_title("git-craft"))
```
In `git-craft/src/render/gpu.rs`, the capability error string:
```rust
"git-craft requires INDIRECT_FIRST_INSTANCE (any Apple Silicon Metal device has it)"
```
In `git-craft/src/mesh/greedy.rs`, the header comment — replace `dabcraft spec` with `git-craft spec`.
In `.gitignore`, replace the comment `# dabcraft runtime data` with `# git-craft runtime data`.

- [ ] **Step 4: Replace the project-name token in CLAUDE.md (preserving the doc path)**

In `CLAUDE.md`: change the heading `# dabcraft` → `# git-craft` and every prose mention of the project name to `git-craft`. **Exception:** the `Key documents` line that references `docs/superpowers/specs/2026-06-11-dabcraft-design.md` keeps that exact path (the filename is not renamed). Also update any `dabcraft/` path mention to `git-craft/`.

- [ ] **Step 5: Replace the token in historical doc bodies (filenames preserved)**

Replace the bare token `dabcraft` with `git-craft` inside the *bodies* of files under `docs/superpowers/specs/` and `docs/superpowers/plans/`. Do this with a path-safe replacement that does not touch filenames. Run from the repo root:
```bash
cd /Users/onurtellioglu/Github/Minecraft
grep -rIl --include='*.md' 'dabcraft' docs/superpowers | while read -r f; do
  perl -0pi -e 's/\bdabcraft\b(?!-design\.md|-m\d|\.md)/git-craft/g' "$f"
done
```
Then manually verify no in-body path reference of the form `docs/superpowers/.../<date>-dabcraft-...md` was broken (those should still read `dabcraft` because they are filenames). If the regex is too blunt for a given file, edit that file by hand instead.

- [ ] **Step 6: Verify the rename leaves only filenames/paths**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft
grep -rIn "dabcraft" . --exclude-dir=target --exclude-dir=.git --exclude-dir=.task-log
```
Expected: every remaining hit is either (a) a dated doc **filename** under `docs/superpowers/`, or (b) a **path reference** to such a filename (e.g. the `CLAUDE.md` Key-documents link, in-body links inside plans/specs). No code, no `Cargo.toml`, no headings. If any other hit remains, fix it.

- [ ] **Step 7: Verify the project still builds and tests pass**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft/git-craft
cargo build --release
cargo test
```
Expected: build succeeds and produces `target/release/git-craft`; all tests pass. Confirm the binary name:
`ls target/release/git-craft` → file exists.

- [ ] **Step 8: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft
git add -A
git commit -m "chore: rename dabcraft to git-craft"
```

---

## Task 3: Add dual MIT/Apache-2.0 license

**Files:**
- Create: `LICENSE-MIT`, `LICENSE-APACHE` (repo root)
- Modify: `git-craft/Cargo.toml`

- [ ] **Step 1: Create `LICENSE-MIT`**

Create `LICENSE-MIT` at the repo root with the standard MIT text:
```text
MIT License

Copyright (c) 2026 Onur Tellioğlu

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 2: Create `LICENSE-APACHE`**

Create `LICENSE-APACHE` at the repo root containing the **verbatim, unmodified** Apache License 2.0. Fetch the canonical text and write it to the file:
```bash
cd /Users/onurtellioglu/Github/Minecraft
curl -fsSL https://www.apache.org/licenses/LICENSE-2.0.txt -o LICENSE-APACHE
```
Verify the first line reads `                                 Apache License` and the file ends with the standard appendix. The "APPENDIX: How to apply the Apache License to your work" boilerplate at the end is part of the canonical text — leave it as-is (do not fill in the bracketed fields; the root LICENSE files cover the whole project).
If `curl` has no network access, paste the canonical Apache-2.0 text manually from a trusted local copy.

- [ ] **Step 3: Add license + package metadata to Cargo.toml**

In `git-craft/Cargo.toml`, expand the `[package]` table. Replace `repository`'s `<gh-login>` with the actual GitHub username from `gh api user --jq .login` (run it; it returns the login string):
```toml
[package]
name = "git-craft"
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
description = "A performance-focused, Minecraft-style voxel engine in Rust on wgpu, targeting modern shader-pack visuals at 384-block render distance / 120 fps on Apple M4."
repository = "https://github.com/<gh-login>/git-craft"
keywords = ["voxel", "gamedev", "wgpu", "renderer", "minecraft"]
categories = ["games", "graphics", "rendering"]
authors = ["Onur Tellioğlu"]
```
> Do **not** add a `readme` field: the README lives at the repo root (above the crate dir) and a `../README.md` path would emit a cargo packaging warning. GitHub renders the root README regardless.

- [ ] **Step 4: Verify metadata parses**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft/git-craft
cargo metadata --no-deps --format-version 1 >/dev/null && echo OK
```
Expected: `OK` (no manifest parse errors).

- [ ] **Step 5: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft
git add LICENSE-MIT LICENSE-APACHE git-craft/Cargo.toml git-craft/Cargo.lock
git commit -m "chore: dual-license under MIT OR Apache-2.0"
```

---

## Task 4: Add README

**Files:**
- Create: `README.md` (repo root)

- [ ] **Step 1: Write `README.md`**

Create `README.md` at the repo root with exactly this content (controls verified against `git-craft/src/app.rs` and `git-craft/src/game/player.rs`):

````markdown
# git-craft

A performance-focused, Minecraft-style voxel engine written in Rust on [wgpu](https://wgpu.rs),
targeting modern shader-pack visuals at a 384-block render distance and 120 fps on Apple M4.

> **Disclaimer:** git-craft is an independent, fan-made project. It is not affiliated with,
> endorsed by, or associated with Mojang Studios or Microsoft. "Minecraft" is a trademark of
> Mojang Synergies AB.

<!-- TODO: add a screenshot or GIF here before going public — first impression matters. -->

## Features

- **Voxel world** — greedy-meshed chunks, infinite streaming worldgen, cave culling.
- **Lighting** — cascaded shadow maps (CSM), flood-fill block/sky light.
- **Modern post-processing** — GTAO, TAA, volumetric in-scatter, bloom, ACES tone mapping.
- **Atmosphere** — Hillaire sky/aerial-perspective LUTs, day/night cycle.
- **Water** — transparent refraction with screen-space reflections and a sky-LUT fallback.
- **Performance discipline** — forward TBDR-friendly pipeline, per-pass GPU timestamps in
  the F3 HUD, a render-scale safety valve, `--bench` percentile reporting.

## Build & Run

Requires a recent Rust toolchain (edition 2024 → Rust ≥ 1.85). The primary tested platform is
macOS on Apple Silicon (Metal).

```bash
cd git-craft
cargo run --release
```

> Release builds only. Debug builds are intentionally slow for voxel work — the `Cargo.toml`
> profile overrides already raise `opt-level` to keep them merely usable, not fast.

## Controls

| Input | Action |
| --- | --- |
| `W` `A` `S` `D` | Move |
| Mouse | Look |
| `Left Ctrl` | Sprint (and fly-sprint) |
| `Space` | Jump (walk) / ascend (fly) |
| Double-tap `Space` or `F` | Toggle walk/fly mode |
| `Left Shift` | Descend (fly mode) |
| Left click | Break block |
| Right click | Place block |
| `1`–`9` | Select hotbar slot |
| Scroll wheel | Cycle hotbar (hold `Shift` to page) |
| `F3` or `H` | Toggle debug HUD |
| `R` | Cycle render scale (1.0 → 0.75 → 0.5) |
| `V` | Toggle cave culling |
| `G` | Toggle GTAO debug view |
| `B` | Toggle volumetric in-scatter debug view |
| `Esc` | Release the mouse (click to re-grab) |

## Architecture

A single Rust binary crate. The engine core (meshing, lighting, worldgen, physics, palette
storage) is written as pure functions over plain data and unit-tested; rendering is a forward
pipeline built for Apple's tile-based deferred (TBDR) GPUs — no Z-prepass, precise load/store
ops, alpha-tested geometry in its own pass after opaque. WGSL shaders live under
`git-craft/assets/shaders/`.

**Tech stack:** wgpu 29, winit, glam, bytemuck, egui, rayon, fastnoise-lite.

## Project status

Early but actively developed. Milestones M1–M5 (foundation, world, playable loop, lighting,
shader-pack visuals) are complete; M6 (persistence, textures, performance) is next.

## Documentation

- Design spec: [`docs/superpowers/specs/2026-06-11-dabcraft-design.md`](docs/superpowers/specs/2026-06-11-dabcraft-design.md)
- Milestone plans: [`docs/superpowers/plans/`](docs/superpowers/plans/)

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) and our
[Code of Conduct](CODE_OF_CONDUCT.md).

## References

Rendering techniques are implemented from published work:

- **ACES** filmic tone mapping — Stephen Hill's fit of the Academy Color Encoding System.
- **Atmosphere** — Sébastien Hillaire, *"A Scalable and Production Ready Sky and Atmosphere
  Rendering Technique"* (EGSR 2020).
- **GTAO** — Jorge Jiménez et al., *"Practical Real-Time Strategies for Accurate Indirect
  Occlusion"* (SIGGRAPH 2016).

## License

Dual-licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option. Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this project shall be dual-licensed as above, without any
additional terms or conditions.
````

- [ ] **Step 2: Verify links resolve**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft
test -f LICENSE-MIT && test -f LICENSE-APACHE && test -f docs/superpowers/specs/2026-06-11-dabcraft-design.md && echo "links OK"
```
Expected: `links OK` (CONTRIBUTING.md and CODE_OF_CONDUCT.md are added in Task 5 — that is fine; this branch will contain them before the PR).

- [ ] **Step 3: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft
git add README.md
git commit -m "docs: add project README"
```

---

## Task 5: Add CONTRIBUTING and CODE_OF_CONDUCT

**Files:**
- Create: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md` (repo root)

- [ ] **Step 1: Write `CONTRIBUTING.md`**

Create `CONTRIBUTING.md` at the repo root with exactly this content:

````markdown
# Contributing to git-craft

Thanks for your interest in contributing! This document explains how to build the project,
the conventions we follow, and how to submit changes.

## Prerequisites

- A recent Rust toolchain — edition 2024 requires Rust ≥ 1.85. Install via [rustup](https://rustup.rs).
- The primary tested platform is macOS on Apple Silicon (Metal). The renderer assumes Metal
  features available on Apple GPUs; other platforms are not yet verified.

## Building and checking

All commands run from the `git-craft/` crate directory:

```bash
cd git-craft
cargo run --release          # play the game (release only — debug is too slow)
cargo test                   # unit tests for the engine core
cargo clippy --all-targets -- -D warnings
cargo fmt                    # format (CI checks with --check)
```

Before opening a pull request, make sure `cargo fmt --check`, `cargo clippy --all-targets --
-D warnings`, and `cargo test` all pass — CI runs exactly these.

## Code conventions

- **Engine core is pure functions.** Meshing, lighting, worldgen, physics, and palette storage
  are written as pure functions over plain data and covered by unit tests. New core logic
  should follow test-driven development.
- **Validate rendering with data, not vibes.** Performance and rendering claims are checked
  with the F3 debug HUD (per-pass GPU timestamps) and `--bench` percentiles — never "by feel".
- **Respect Apple TBDR discipline.** Forward rendering, no Z-prepass, precise load/store ops,
  alpha-tested geometry in its own pipeline after solid opaque.
- WGSL shaders live under `git-craft/assets/shaders/`.

## Assets and intellectual property

Do **not** contribute Minecraft (or any other proprietary) assets — textures, sounds, models,
or data files. Any art or audio you add must be your own original work or licensed under a
compatible permissive / Creative Commons license, with attribution noted in your PR.

## Git workflow

- Work on a feature branch — never commit directly to `main`.
- Keep commits atomic (one logical change each).
- Commit message format: `type: what and why`, where `type` is one of `feat`, `fix`,
  `refactor`, `chore`, `docs`, `test`.
- Branch names: `<type>/<short-kebab-description>` — e.g. `feat/chunk-saving`,
  `fix/water-z-fighting`.

## Pull requests

1. Fork the repo and create your branch from `main`.
2. Make your change; ensure fmt + clippy + tests pass.
3. Open a pull request using the template, link any related issue, and describe how you
   verified the change (HUD readings / benchmark numbers for rendering work).

## License of contributions

By contributing, you agree that your contributions are dual-licensed under the MIT and
Apache-2.0 licenses (inbound = outbound), with no additional terms and no CLA.
````

- [ ] **Step 2: Write `CODE_OF_CONDUCT.md`**

Create `CODE_OF_CONDUCT.md` at the repo root with the **Contributor Covenant 2.1**. Fetch the
canonical Markdown and adjust only the enforcement-contact line:
```bash
cd /Users/onurtellioglu/Github/Minecraft
curl -fsSL https://raw.githubusercontent.com/ContributorCovenant/contributor_covenant/release/content/version/2/1/code_of_conduct.md -o CODE_OF_CONDUCT.md
```
Then edit the **Enforcement** section: replace the placeholder contact line (the canonical
text contains `[INSERT CONTACT METHOD]`) with:
> Instances of abusive, harassing, or otherwise unacceptable behavior may be reported to the
> project maintainers by opening an issue on the project's GitHub repository.

**Do not publish any personal email address.** Reporting goes through GitHub Issues only.
If `curl` has no network access, write the Contributor Covenant 2.1 text from a trusted local
copy and apply the same enforcement-contact edit.

- [ ] **Step 3: Verify the contact edit**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft
grep -n "INSERT CONTACT METHOD" CODE_OF_CONDUCT.md && echo "STILL HAS PLACEHOLDER — FIX" || echo "contact OK"
grep -ni "@" CODE_OF_CONDUCT.md | grep -iE "mail|icloud|gmail" && echo "EMAIL FOUND — REMOVE" || echo "no email OK"
```
Expected: `contact OK` and `no email OK`.

- [ ] **Step 4: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft
git add CONTRIBUTING.md CODE_OF_CONDUCT.md
git commit -m "docs: add contributing guide and code of conduct"
```

---

## Task 6: Add GitHub templates and CI workflow

**Files:**
- Create: `.github/ISSUE_TEMPLATE/bug_report.yml`, `.github/ISSUE_TEMPLATE/feature_request.yml`, `.github/ISSUE_TEMPLATE/config.yml`, `.github/PULL_REQUEST_TEMPLATE.md`, `.github/workflows/ci.yml`

- [ ] **Step 1: Write `.github/ISSUE_TEMPLATE/bug_report.yml`**

```yaml
name: Bug report
description: Report a crash, rendering glitch, or incorrect behavior
labels: ["bug"]
body:
  - type: textarea
    id: description
    attributes:
      label: Description
      description: What happened, and what did you expect instead?
    validations:
      required: true
  - type: textarea
    id: repro
    attributes:
      label: Steps to reproduce
      placeholder: |
        1. Launch the game
        2. ...
        3. See the issue
    validations:
      required: true
  - type: textarea
    id: environment
    attributes:
      label: Environment
      description: OS, GPU / Apple Silicon model, Rust version (`rustc --version`), render-scale setting, and the F3 HUD readout if relevant.
      placeholder: |
        - OS: macOS 15.5
        - GPU: Apple M4
        - rustc: 1.85.0
        - Render scale: 1.0
    validations:
      required: true
  - type: textarea
    id: screenshots
    attributes:
      label: Screenshots
      description: Drag images here for rendering issues (optional).
    validations:
      required: false
```

- [ ] **Step 2: Write `.github/ISSUE_TEMPLATE/feature_request.yml`**

```yaml
name: Feature request
description: Suggest an idea or enhancement
labels: ["enhancement"]
body:
  - type: textarea
    id: problem
    attributes:
      label: Problem
      description: What problem or limitation are you running into?
    validations:
      required: true
  - type: textarea
    id: solution
    attributes:
      label: Proposed solution
      description: What would you like to see happen?
    validations:
      required: true
  - type: textarea
    id: alternatives
    attributes:
      label: Alternatives considered
    validations:
      required: false
```

- [ ] **Step 3: Write `.github/ISSUE_TEMPLATE/config.yml`**

```yaml
blank_issues_enabled: true
```

- [ ] **Step 4: Write `.github/PULL_REQUEST_TEMPLATE.md`**

```markdown
## Summary

<!-- What does this PR change and why? -->

## Related issue

<!-- e.g. Closes #123 -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / cleanup
- [ ] Documentation

## Checklist

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] Follows the conventions in CONTRIBUTING.md
- [ ] For rendering/perf changes: verified via the F3 HUD / `--bench` (numbers in the PR)
```

- [ ] **Step 5: Write `.github/workflows/ci.yml`**

The crate is in the `git-craft/` subdirectory, so the job sets a working directory.

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  check:
    name: fmt + clippy + test
    runs-on: macos-latest
    defaults:
      run:
        working-directory: git-craft
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: git-craft
      - name: Format
        run: cargo fmt --check
      - name: Clippy
        run: cargo clippy --all-targets -- -D warnings
      - name: Test
        run: cargo test
```

- [ ] **Step 6: Validate the YAML parses**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft
for f in .github/ISSUE_TEMPLATE/*.yml .github/workflows/ci.yml; do
  ruby -ryaml -e "YAML.load_file('$f')" 2>/dev/null && echo "OK $f" || python3 -c "import yaml,sys; yaml.safe_load(open('$f'))" && echo "OK $f"
done
```
Expected: `OK` for each file. (If neither `ruby` nor `python3 -m yaml` is available, visually confirm indentation instead.)

- [ ] **Step 7: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft
git add .github
git commit -m "chore: add issue/PR templates and CI workflow"
```

---

## Task 7: Final verification, publish, and open a PR

**Files:** none (verification + git/GitHub operations)

> **Outward-facing step ahead.** Creating the GitHub repository is irreversible-ish (even a
> private repo persists history). Get the maintainer's explicit go-ahead before Step 3.

- [ ] **Step 1: Full local verification**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft/git-craft
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release && ls target/release/git-craft
```
Expected: fmt clean, clippy clean (no warnings), tests pass, and a `git-craft` binary exists.

- [ ] **Step 2: Confirm the file inventory and branch state**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft
ls README.md LICENSE-MIT LICENSE-APACHE CONTRIBUTING.md CODE_OF_CONDUCT.md
ls .github/ISSUE_TEMPLATE/bug_report.yml .github/ISSUE_TEMPLATE/feature_request.yml .github/PULL_REQUEST_TEMPLATE.md .github/workflows/ci.yml
git log --oneline main..chore/open-source-setup
```
Expected: all files present; the log shows the five Task 2–6 commits.

- [ ] **Step 3: Create the private GitHub repo and push (maintainer go-ahead required)**

From the repo root, create the repo as **private** and push all branches:
```bash
cd /Users/onurtellioglu/Github/Minecraft
gh repo create git-craft --private --source=. --remote=origin --description "A performance-focused, Minecraft-style voxel engine in Rust on wgpu."
git push -u origin main
git push -u origin chore/open-source-setup
```
Expected: repo `git-craft` created under the authenticated account; both branches pushed. Note the repo URL from the `gh` output.

- [ ] **Step 4: Open the pull request**

```bash
cd /Users/onurtellioglu/Github/Minecraft
gh pr create --base main --head chore/open-source-setup \
  --title "chore: open-source setup (rename to git-craft, license, contribution infra)" \
  --body "Renames the project to git-craft, dual-licenses MIT/Apache-2.0, and adds README, CONTRIBUTING, CODE_OF_CONDUCT, issue/PR templates, and CI. Spec: docs/superpowers/specs/2026-06-14-git-craft-open-source-design.md"
```
Expected: a PR is opened from `chore/open-source-setup` into `main`. Note the PR URL.

- [ ] **Step 5: Confirm CI runs**

Run: `gh pr checks --watch` (or check the PR page).
Expected: the `CI / fmt + clippy + test` workflow runs and passes on `macos-latest`. If it fails, read the logs, fix on the branch, and push.

- [ ] **Step 6: Hand off to the maintainer**

Report the repo URL and PR URL. Remind the maintainer of the two manual actions that are
intentionally **not** automated:
1. Review and merge the PR (`chore/open-source-setup` → `main`).
2. Flip the repository from **private** to **public** when satisfied
   (Settings → General → Danger Zone → Change visibility), and add a screenshot/GIF to the
   README (the `<!-- TODO -->` placeholder).

---

## Self-Review

**Spec coverage:**
- §3 Rename → Task 2 (dir move, Cargo, code strings, docs token, preserve-list, grep + build verify). ✓
- §4 Licensing → Task 3 (LICENSE-MIT, LICENSE-APACHE, Cargo metadata, inbound=outbound stated in README/CONTRIBUTING). ✓
- §5.1 README → Task 4 (all listed sections, verified controls table, References). ✓
- §5.2 CONTRIBUTING → Task 5 Step 1. ✓
- §5.3 CODE_OF_CONDUCT (Issues contact, no email) → Task 5 Steps 2–3. ✓
- §5.4 issue templates / §5.5 PR template / §5.6 CI (macos-latest) → Task 6. ✓
- §6 Trademark disclaimer + asset rule → README (Task 4) + CONTRIBUTING (Task 5). ✓
- §7 Publishing flow (pre-flight, merge, branch, atomic commits, private repo, PR, handoff) → Tasks 1, 7, and per-task commits. ✓
- §8 Success criteria → Task 7 Steps 1–2 verification. ✓

**Placeholder scan:** The only intentional placeholders are `<gh-login>` in Cargo.toml
(resolved via `gh api user --jq .login` in Task 3 Step 3) and the README `<!-- TODO -->`
screenshot marker (a deliverable placeholder the maintainer fills, flagged in Task 7 Step 6).
No "TBD"/"implement later" steps; every content step shows the full content.

**Type/name consistency:** `git-craft` (package, binary, dir, prose) used uniformly; commit
types match the project's `feat/fix/refactor/chore/docs` set; the `git-craft/` working
directory is used consistently in every cargo command and in the CI `working-directory`.

