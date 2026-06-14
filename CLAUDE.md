# CLAUDE.md

Read **[AGENTS.md](AGENTS.md)** first — canonical project guide (architecture map, conventions, guardrails). This file only surfaces the few traps not obvious from the tree.

## Must-follow constraints

- **Run every `cargo` command from `git-craft/`, not the repo root** — the root has no `Cargo.toml`; the crate is the `git-craft/` subdir. (Working dir defaults to the repo root.)
- **Always run/play with `--release`** — debug builds are unusably slow for voxel work even at the configured `opt-level = 1`.
- **No proprietary assets** — no Minecraft (or other) textures, sounds, or data. Art/audio must be original or under a permissive/CC license.
- Crate is **edition 2024** — needs a current stable toolchain.

## Validation before finishing (from `git-craft/`)

```bash
cargo fmt                                    # CI gate — CI runs --check
cargo clippy --all-targets -- -D warnings    # must be clean
cargo test                                   # engine-core unit tests
```

- Add an `[Unreleased]` entry to `CHANGELOG.md` for any behavior-affecting change.

## Repo-specific conventions

- Engine core (meshing, lighting, worldgen, physics, palette storage) is pure functions over plain data with no I/O — write the test first for changes here.
- Justify perf/visual claims with the **F3 HUD** (per-pass GPU timestamps) and `cargo run --release -- --bench` percentiles — never "by feel."
- Apple TBDR renderer: forward, **no Z-prepass**, alpha-tested geometry in its own pipeline after solid opaque. Do not reorder passes or add a Z-prepass without measured justification.

## Important locations

- Design spec: `docs/superpowers/specs/2026-06-11-dabcraft-design.md`; milestone plans: `docs/superpowers/plans/`.
- Shaders: `git-craft/assets/shaders/*.wgsl` — hot-reloaded at runtime.
- `/contribute` project command (`.claude/commands/`) runs the guided contribution loop.
