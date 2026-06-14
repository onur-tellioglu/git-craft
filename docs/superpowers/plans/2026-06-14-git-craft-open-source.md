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
  - VISION.md
  - CHANGELOG.md
  - AGENTS.md
  - SECURITY.md
  - LICENSE-MIT
  - LICENSE-APACHE
  - CONTRIBUTING.md
  - CODE_OF_CONDUCT.md
  - .github/**
  - .claude/**
  - docs/superpowers/**
trigger-tasks-touched: []
shared-modules-touched: []
---

# git-craft Open-Source Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the project to `git-craft`, dual-license it MIT/Apache-2.0, bake in its community-and-agent-driven mission, add contribution + governance + agent-enablement infrastructure with versioning, and publish to GitHub as a private repo ready to be flipped public with a protected `main`.

**Architecture:** A rename pass (directory, Cargo package, code strings, docs token) followed by additive root/`.github`/`.claude` files — license, README (with mission), VISION, CHANGELOG, CONTRIBUTING, CoC, SECURITY, AGENTS, CODEOWNERS, issue/PR templates, CI + release workflows, and a project-local `/contribute` command — then a git publishing flow (merge latest code to `main`, atomic commits on a `chore/` branch, create the repo, push, set topics, open a PR, and configure branch protection after the public flip).

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

## Why git-craft?

Minecraft's direction no longer matches what many players and builders want, and the
community has always been able to dream bigger than the official roadmap — but it could
never touch the engine itself. git-craft is the other path: a voxel sandbox that is
**open to the engine, built in public, and grown by its community**. And because AI coding
agents have matured, contributing no longer requires deep prior knowledge of the codebase —
point your agent at our docs and guardrails and it can carry real work. This is an
experiment in what a community can build on its own, with agents as force-multipliers. Read
the full story in [VISION.md](VISION.md) — then come build with us.

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

Documentation is a permanent, first-class part of this repo.

- Vision & mission: [VISION.md](VISION.md)
- Changelog: [CHANGELOG.md](CHANGELOG.md)
- Design spec: [`docs/superpowers/specs/2026-06-11-dabcraft-design.md`](docs/superpowers/specs/2026-06-11-dabcraft-design.md)
- Milestone plans: [`docs/superpowers/plans/`](docs/superpowers/plans/)

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) and our
[Code of Conduct](CODE_OF_CONDUCT.md). This project is built to be developed *with AI
agents*: if you use one, point it at [AGENTS.md](AGENTS.md) first.

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

## Building with an AI agent

git-craft is explicitly designed to be developed with AI coding agents (Claude Code and
others). You do not need deep prior knowledge of the codebase to contribute meaningfully.

- Point your agent at [AGENTS.md](AGENTS.md) first — it is the canonical, tool-agnostic
  guide to building, testing, the architecture map, and the project's hard conventions.
- Give it the relevant design context: the design spec under `docs/superpowers/specs/` and
  the milestone plan you are working from under `docs/superpowers/plans/`.
- Claude Code users: this repo ships a `/contribute` command (in `.claude/commands/`) that
  walks your agent through the full loop.
- Keep the loop tight: small feature branch → test-first change → `cargo fmt` + `clippy` +
  `test` green → open a PR. CI and the maintainer's review are the safety net; let them gate
  you rather than landing large unreviewed changes.

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

## Versioning & releases

git-craft follows [Semantic Versioning](https://semver.org). The version in
`git-craft/Cargo.toml` is the single source of truth; while we are on `0.x`, expect breaking
changes between minor versions.

- Every behavior-affecting PR adds a bullet under the `[Unreleased]` heading in
  [CHANGELOG.md](CHANGELOG.md) (Added / Changed / Fixed / Removed).
- Cutting a release (maintainer): move `[Unreleased]` entries under a new
  `[X.Y.Z] - YYYY-MM-DD` heading, bump `Cargo.toml`, commit, then tag and push:
  `git tag vX.Y.Z && git push origin vX.Y.Z`. The release workflow turns the tag into a
  GitHub Release using that changelog section.

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

## Task 6: Add GitHub templates, governance files, and CI/release workflows

**Files:**
- Create: `.github/ISSUE_TEMPLATE/bug_report.yml`, `.github/ISSUE_TEMPLATE/feature_request.yml`, `.github/ISSUE_TEMPLATE/config.yml`, `.github/PULL_REQUEST_TEMPLATE.md`, `.github/CODEOWNERS`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `SECURITY.md`

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

- [ ] **Step 6: Write `.github/CODEOWNERS`**

Routes every PR's review request to the maintainer. Replace `<gh-login>` with the GitHub
username from `gh api user --jq .login`:
```text
# The maintainer owns the whole tree; all PRs request their review.
*       @<gh-login>
```

- [ ] **Step 7: Write `SECURITY.md`**

Create `SECURITY.md` at the repo root:
```markdown
# Security Policy

git-craft is an early-stage, best-effort project. We still take security seriously.

## Reporting a vulnerability

Please report security issues **privately** rather than opening a public issue:

- Preferred: open a private report via this repository's **Security → Advisories →
  Report a vulnerability** (GitHub Security Advisories).
- We will acknowledge the report and respond as soon as we reasonably can.

## Supported versions

The project is pre-1.0; only the latest `main` and the most recent tagged release receive
fixes. There is no long-term-support guarantee yet.
```

- [ ] **Step 8: Write `.github/workflows/release.yml`**

On a pushed `v*` tag, create a GitHub Release whose notes are the matching `CHANGELOG.md`
section:
```yaml
name: Release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Extract changelog section
        id: notes
        run: |
          VERSION="${GITHUB_REF_NAME#v}"
          awk -v ver="$VERSION" '
            /^## \[/ { if (found) exit }
            $0 ~ "^## \\[" ver "\\]" { found=1 }
            found { print }
          ' CHANGELOG.md > release-notes.md
          if [ ! -s release-notes.md ]; then
            echo "No changelog section for $VERSION" > release-notes.md
          fi
      - name: Create GitHub Release
        env:
          GH_TOKEN: ${{ github.token }}
        run: gh release create "$GITHUB_REF_NAME" --title "$GITHUB_REF_NAME" --notes-file release-notes.md
```

- [ ] **Step 9: Validate the YAML parses**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft
for f in .github/ISSUE_TEMPLATE/*.yml .github/workflows/*.yml; do
  ruby -ryaml -e "YAML.load_file('$f')" 2>/dev/null && echo "OK $f" || python3 -c "import yaml,sys; yaml.safe_load(open('$f'))" && echo "OK $f"
done
```
Expected: `OK` for each file. (If neither `ruby` nor `python3 -m yaml` is available, visually confirm indentation instead.)

- [ ] **Step 10: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft
git add .github SECURITY.md
git commit -m "chore: add issue/PR templates, CODEOWNERS, security policy, and CI/release workflows"
```

---

## Task 7: Add VISION.md (the manifesto)

**Files:**
- Create: `VISION.md` (repo root)

- [ ] **Step 1: Write `VISION.md`**

Create `VISION.md` at the repo root with exactly this content:

````markdown
# Vision

## Why this exists

Minecraft proved that a voxel sandbox can be a canvas for millions of imaginations. But the
game belongs to a single studio, and its direction — shaped by Mojang and Microsoft — no
longer matches what many of the people who love it actually want. The community has always
dreamed bigger than the official roadmap: mods, shaders, data packs, total conversions. Yet
the one thing the community could never touch was the **engine** itself.

git-craft takes the other road. It is a voxel sandbox that is **open all the way down** —
the renderer, the world generation, the physics, the lighting — and it grows by
contribution, not by a corporate roadmap.

## The agent-era thesis

Something changed recently: AI coding agents got good. Tools like Claude Code are now in the
hands of ordinary developers, and a well-scoped change to an unfamiliar codebase no longer
requires weeks of ramp-up. If the project provides clear documentation and firm guardrails,
a contributor — guided by their agent — can land real, correct work on day one.

That reframes what a community project can be. git-craft is an experiment in exactly this:

> **What can a community build on its own, in the open, with AI agents as
> force-multipliers?**

We want to find out — and to show it.

## Principles

- **Open by default.** The engine, the docs, the design history — all public, all permanent.
- **Documentation is first-class.** Specs and plans live in the repo forever. If it isn't
  written down, an agent can't build from it, and neither can a newcomer.
- **Built for agents and humans alike.** `AGENTS.md` and a guided `/contribute` flow mean a
  contributor's tooling doesn't have to match the maintainer's.
- **Small, reviewed steps.** Changes land through PRs with green CI and human review on a
  protected `main`. Trust the process, not heroics.
- **Validate with data, not vibes.** Performance and rendering claims are proven with the
  F3 HUD timestamps and benchmarks — never asserted by feel.
- **Performance is a feature.** The target is shader-pack-grade visuals at a 384-block
  render distance and 120 fps on Apple M4.

## Where we are

Early, and honest about it. The engine already does cascaded shadows, GTAO, TAA,
volumetrics, bloom, ACES tone mapping, a Hillaire atmosphere, and screen-space water
reflections. There is no persistence or texturing yet. The roadmap is in the open under
`docs/superpowers/`.

## Come build with us

If any of this resonates, you are exactly who this project is for. Read
[CONTRIBUTING.md](CONTRIBUTING.md), point your agent at [AGENTS.md](AGENTS.md), and open a
PR. Let's show what a community can do.
````

- [ ] **Step 2: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft
git add VISION.md
git commit -m "docs: add VISION manifesto"
```

---

## Task 8: Add CHANGELOG.md

**Files:**
- Create: `CHANGELOG.md` (repo root)

- [ ] **Step 1: Write `CHANGELOG.md`**

Create `CHANGELOG.md` at the repo root with exactly this content:

````markdown
# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the project is on `0.x`, breaking changes may occur between minor versions.

## [Unreleased]

### Added

- Open-source release: dual MIT/Apache-2.0 license, README, VISION, CONTRIBUTING,
  CODE_OF_CONDUCT, SECURITY, AGENTS guide, issue/PR templates, and CI.

## [0.1.0] - 2026-06-14

### Added

- Voxel world: greedy-meshed chunks, infinite streaming worldgen, cave culling.
- Lighting: cascaded shadow maps (CSM), flood-fill block/sky light.
- Post-processing: GTAO, TAA, volumetric in-scatter, bloom, ACES tone mapping.
- Atmosphere: Hillaire sky and aerial-perspective LUTs, day/night cycle.
- Water: transparent refraction with screen-space reflections and a sky-LUT fallback.
- Performance tooling: F3 debug HUD with per-pass GPU timestamps, render-scale safety
  valve, `--bench` percentile reporting.
````

> **Note on the date:** `2026-06-14` is the date this changelog baseline is written. If the
> implementation runs on a different day, set the `[0.1.0]` date to the actual date — do not
> use a placeholder.

- [ ] **Step 2: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft
git add CHANGELOG.md
git commit -m "docs: add changelog with 0.1.0 baseline"
```

---

## Task 9: Add agent enablement (AGENTS.md, CLAUDE.md pointer, .claude/)

**Files:**
- Create: `AGENTS.md` (repo root), `.claude/commands/contribute.md`, `.claude/settings.json`
- Modify: `CLAUDE.md` (repo root), `.gitignore`

- [ ] **Step 1: Write `AGENTS.md`** (canonical, tool-agnostic agent guide)

Create `AGENTS.md` at the repo root with exactly this content:

````markdown
# AGENTS.md

Guidance for AI coding agents (and humans) working on **git-craft**. This is the canonical
project guide — read it before making changes. See [VISION.md](VISION.md) for why the
project exists.

## What this is

A performance-focused, Minecraft-style voxel engine in Rust on wgpu, targeting shader-pack
visuals at a 384-block render distance / 120 fps on Apple M4. Single binary crate in the
`git-craft/` subdirectory.

## Build, run, test (run from `git-craft/`)

```bash
cd git-craft
cargo run --release                          # play (release only — debug is too slow)
cargo run --release -- --bench               # benchmark; prints frame-time percentiles
cargo test                                   # unit tests for the engine core
cargo clippy --all-targets -- -D warnings    # lints (must be clean)
cargo fmt                                     # format (CI checks with --check)
```

## Architecture map

- `src/main.rs` — entry point.
- `src/app.rs` — winit application: event loop, input handling, per-frame orchestration,
  render-pass wiring, keybindings.
- `src/game/` — gameplay and simulation: `camera`, `player`, `physics`, `raycast`,
  `input`, `hotbar`, `daycycle`.
- `src/mesh/` — chunk meshing (pure functions): `greedy` mesher, `neighborhood`, `padded`,
  `quad`.
- `src/world/` — world data and generation: `block`, `chunks`, `section` (palette storage),
  `gen`, `decor`, `light` / `light_engine`, `jobs` (streaming).
- `src/render/` — wgpu pipelines and passes: `gpu`, `targets`, `terrain`, `shadow`, `gtao`,
  `taa`, `volumetric`, `bloom`, `exposure`, `post`, `atmosphere`, `water`, `outline`,
  `frustum`, `visibility`, `timestamps`, `egui_layer`, `game_ui`, `hot_reload`, `arena`.
- `assets/shaders/*.wgsl` — WGSL shaders, loaded (and hot-reloadable) at runtime.

## Conventions (hard rules)

- **Engine core is pure functions over plain data, unit-tested (TDD).** Meshing, lighting,
  worldgen, physics, and palette storage have no I/O and are covered by `cargo test`. Add
  tests first for changes here.
- **Validate rendering with data, not feel.** Use the F3 debug HUD (per-pass GPU
  timestamps) and `--bench` percentiles to justify performance/visual claims.
- **Apple TBDR discipline.** Forward rendering, no Z-prepass, precise load/store ops,
  alpha-tested geometry in its own pipeline after solid opaque. Don't add a Z-prepass or
  break the pass ordering without strong measured justification.

## Guardrails

- **Never push to `main`.** It is protected; land changes via a PR from a feature branch.
- **No proprietary assets.** Do not add Minecraft (or other proprietary) textures, sounds,
  or data. Art/audio must be original or under a compatible permissive/CC license.
- **Keep PRs small and focused.** Run `cargo fmt` + `clippy` + `test` before opening one.
- **Read the design first.** The spec is `docs/superpowers/specs/2026-06-11-dabcraft-design.md`;
  milestone plans are in `docs/superpowers/plans/`.

## Git & PR conventions

- Feature branches named `<type>/<short-kebab>`; commits `type: what and why`
  (`feat`/`fix`/`refactor`/`chore`/`docs`/`test`); atomic commits.
- Add a `[Unreleased]` line to `CHANGELOG.md` for behavior-affecting changes.
- Open PRs with the template; CI (fmt + clippy + test on macOS) must pass.
````

- [ ] **Step 2: Rewrite `CLAUDE.md` as a pointer + quick reference**

Replace the entire contents of `CLAUDE.md` (repo root) with:

````markdown
# git-craft — Claude Code notes

This project's canonical guidance lives in **[AGENTS.md](AGENTS.md)** — read it first. It
covers the architecture map, conventions, and guardrails.

## Quick reference (run from `git-craft/`)

```bash
cargo run --release                          # play
cargo test                                   # engine-core unit tests
cargo clippy --all-targets -- -D warnings    # lints
cargo fmt                                     # format
```

## Guardrails

- Never push to `main` — land changes via a PR from a feature branch.
- No proprietary assets. Validate rendering/perf with the F3 HUD and `--bench`, not by feel.
- Read the design spec at `docs/superpowers/specs/2026-06-11-dabcraft-design.md` first.

## Claude Code specifics

- Run `/contribute` (project command in `.claude/commands/`) for a guided contribution loop.
- Safe, common commands are pre-allowed in `.claude/settings.json`.
````

> This supersedes the earlier project-overview content of `CLAUDE.md`; that material now
> lives, expanded, in `AGENTS.md`.

- [ ] **Step 3: Write `.claude/commands/contribute.md`**

```markdown
---
description: Guided contribution flow for git-craft (read → branch → TDD → checks → PR)
---

You are contributing to git-craft, a community- and agent-driven voxel game engine.
Follow this loop. Never push to `main` — all changes land via PR.

1. Read `AGENTS.md` (the canonical project guide) and skim the design spec at
   `docs/superpowers/specs/2026-06-11-dabcraft-design.md`. If a milestone plan under
   `docs/superpowers/plans/` covers the work, read it too.
2. Confirm scope with the user: $ARGUMENTS. If it is unclear, ask one focused question.
3. Create a feature branch: `git checkout -b <type>/<short-kebab>` (type = feat/fix/refactor/chore/docs).
4. Implement test-first for engine-core changes (pure functions in `src/mesh`, `src/world`,
   `src/game`): write a failing `cargo test`, then make it pass. For rendering work,
   validate with the F3 HUD / `--bench`, not by feel.
5. From `git-craft/`, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and
   `cargo test`. Fix anything red.
6. If behavior changed, add a line under `[Unreleased]` in `CHANGELOG.md`.
7. Commit atomically (`type: what and why`), then open a PR with `gh pr create` using the
   template. Describe how you verified the change.

Keep PRs small and focused. CI and maintainer review are the safety net.
```

- [ ] **Step 4: Write `.claude/settings.json`** (minimal safe allowlist)

```json
{
  "permissions": {
    "allow": [
      "Bash(cargo build:*)",
      "Bash(cargo run:*)",
      "Bash(cargo test:*)",
      "Bash(cargo clippy:*)",
      "Bash(cargo fmt:*)",
      "Bash(cargo check:*)",
      "Bash(git status:*)",
      "Bash(git diff:*)",
      "Bash(git add:*)",
      "Bash(git commit:*)",
      "Bash(git checkout:*)",
      "Bash(git switch:*)",
      "Bash(git branch:*)",
      "Bash(git log:*)",
      "Bash(gh pr:*)",
      "Bash(gh issue:*)"
    ]
  }
}
```

> Deliberately excludes destructive/outward commands (`git push`, `git reset`, `rm`, repo
> deletion). Contributors keep full control over anything risky.

- [ ] **Step 5: Ignore local Claude settings overrides**

Append to `.gitignore` (so personal local overrides are never committed, but the shared
`settings.json` and commands are):
```text

# Claude Code local overrides
.claude/settings.local.json
```

- [ ] **Step 6: Verify the JSON parses**

Run:
```bash
cd /Users/onurtellioglu/Github/Minecraft
python3 -c "import json; json.load(open('.claude/settings.json'))" && echo "settings.json OK"
```
Expected: `settings.json OK`.

- [ ] **Step 7: Commit**

```bash
cd /Users/onurtellioglu/Github/Minecraft
git add AGENTS.md CLAUDE.md .claude .gitignore
git commit -m "docs: add AGENTS guide and project-local Claude contribution tooling"
```

---

## Task 10: Final verification, publish, and open a PR

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
ls README.md VISION.md CHANGELOG.md AGENTS.md CLAUDE.md SECURITY.md LICENSE-MIT LICENSE-APACHE CONTRIBUTING.md CODE_OF_CONDUCT.md
ls .github/ISSUE_TEMPLATE/bug_report.yml .github/ISSUE_TEMPLATE/feature_request.yml .github/PULL_REQUEST_TEMPLATE.md .github/CODEOWNERS .github/workflows/ci.yml .github/workflows/release.yml
ls .claude/commands/contribute.md .claude/settings.json
git log --oneline main..chore/open-source-setup
```
Expected: all files present; the log shows the eight Task 2–9 commits (rename, license,
README, CONTRIBUTING+CoC, .github, VISION, CHANGELOG, agent tooling).

- [ ] **Step 3: Create the private GitHub repo and push (maintainer go-ahead required)**

From the repo root, create the repo as **private**, push all branches, and set the topics:
```bash
cd /Users/onurtellioglu/Github/Minecraft
gh repo create git-craft --private --source=. --remote=origin \
  --description "An open, community- and AI-agent-driven Minecraft-style voxel engine in Rust on wgpu."
git push -u origin main
git push -u origin chore/open-source-setup
gh repo edit --add-topic voxel,game-engine,rust,wgpu,sandbox,open-source,ai-agents,community-driven
```
Expected: repo `git-craft` created under the authenticated account; both branches pushed; topics set. Note the repo URL from the `gh` output.

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

Report the repo URL and PR URL. Remind the maintainer of the manual actions that are
intentionally **not** automated, in order:
1. Review and merge the PR (`chore/open-source-setup` → `main`).
2. Flip the repository from **private** to **public** when satisfied
   (Settings → General → Danger Zone → Change visibility), and add a screenshot/GIF to the
   README (the `<!-- TODO -->` placeholder).
3. **Protect `main`** (do this once public — branch protection is free for public repos; on
   a free plan it does not apply while private). Either via the UI
   (Settings → Branches → Add branch ruleset / protection rule for `main`) with: *require a
   pull request before merging*, *require 1 approval*, *require status checks to pass* →
   select `fmt + clippy + test`, *require branches up to date*, *block force pushes*, *block
   deletions* — or via the API:
   ```bash
   OWNER=$(gh api user --jq .login)
   gh api -X PUT "repos/$OWNER/git-craft/branches/main/protection" \
     -H "Accept: application/vnd.github+json" \
     --input - <<'JSON'
   {
     "required_status_checks": { "strict": true, "contexts": ["fmt + clippy + test"] },
     "enforce_admins": false,
     "required_pull_request_reviews": { "required_approving_review_count": 1 },
     "restrictions": null
   }
   JSON
   ```
   Verify: `gh api "repos/$OWNER/git-craft/branches/main/protection" --jq '.required_pull_request_reviews, .required_status_checks'` returns the configured rules.

---

## Self-Review

**Spec coverage:**
- §3 Rename → Task 2 (dir move, Cargo, code strings, docs token, preserve-list, grep + build verify). ✓
- §4 Licensing → Task 3 (LICENSE-MIT, LICENSE-APACHE, Cargo metadata, inbound=outbound stated in README/CONTRIBUTING/CHANGELOG). ✓
- §5.1 README → Task 4 (all listed sections, "Why git-craft?", verified controls table, References, doc links). ✓
- §5.2 CONTRIBUTING → Task 5 (build/conventions/git/PR + "Building with an AI agent" + "Versioning & releases"). ✓
- §5.3 CODE_OF_CONDUCT (Issues contact, no email) → Task 5 Steps 2–3. ✓
- §5.4 issue templates / §5.5 PR template / §5.6 CI (macos-latest) → Task 6 Steps 1–5, 9–10. ✓
- §6 Trademark disclaimer + asset rule → README (Task 4) + CONTRIBUTING + AGENTS (Tasks 5, 9). ✓
- §7 Publishing flow (pre-flight, merge, branch, atomic commits, private repo, topics, PR, handoff) → Tasks 1, 10. ✓
- §8 Success criteria → Task 10 Steps 1–2 verification. ✓
- §10 Vision artifacts → Task 7 (VISION.md), Task 4 ("Why git-craft?"), Task 10 Step 3 (description/topics). ✓
- §11 Versioning & releases → Task 8 (CHANGELOG), Task 5 ("Versioning & releases"), Task 6 Step 8 (release.yml). ✓
- §12 Governance → Task 6 Steps 6–7 (CODEOWNERS, SECURITY.md), Task 10 Step 6.3 (branch protection). ✓
- §13 Agent enablement → Task 9 (AGENTS.md, CLAUDE.md pointer, `.claude/commands/contribute.md`, `.claude/settings.json`) + Task 5 AI section. ✓

**Placeholder scan:** Intentional, resolved placeholders only: `<gh-login>` in Cargo.toml /
CODEOWNERS / branch-protection (resolved via `gh api user --jq .login`); the README
`<!-- TODO -->` screenshot marker (maintainer fills it, flagged in Task 10 Step 6); the
CHANGELOG `[0.1.0]` date (Task 8 notes to use the real run date). No "TBD"/"implement later"
steps; every content step shows full content.

**Type/name consistency:** `git-craft` (package, binary, dir, prose) used uniformly; the CI
job name `fmt + clippy + test` matches the branch-protection required status-check context in
Task 10 Step 6; commit types match `feat/fix/refactor/chore/docs`; the `git-craft/` working
directory is used consistently in every cargo command and in the CI `working-directory`;
`AGENTS.md` is referenced as canonical from README, CONTRIBUTING, CLAUDE.md, and `/contribute`.

