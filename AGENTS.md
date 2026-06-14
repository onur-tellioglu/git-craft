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
  `frustum`, `visibility`, `timestamps`, `egui_layer`, `game_ui`, `hot_reload`, `arena`,
  `material` (procedurally-generated per-block albedo/normal/roughness texture arrays).
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
