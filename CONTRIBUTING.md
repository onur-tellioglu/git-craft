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
