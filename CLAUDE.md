# dabcraft

Performance-focused Minecraft-style voxel game in Rust on wgpu, targeting modern shader-pack visuals (CSM shadows, GTAO, volumetrics, bloom, ACES, Hillaire atmosphere) at 384-block render distance / 120 fps on Apple M4.

## Key documents

- Design spec: `docs/superpowers/specs/2026-06-11-dabcraft-design.md` — read before any engine work.

## Repo decisions

- `CLAUDE.md` and `docs/` are committed to this repo.
- Single binary crate (`dabcraft/` once scaffolded), WGSL shaders under `assets/shaders/`.

## Conventions

- Engine core (meshing, lighting, worldgen, physics, palette storage) is pure functions over plain data — unit tested, TDD.
- Rendering claims are validated with the F3 debug HUD (per-pass GPU timestamps) and `--bench` percentiles, never by feel.
- Apple TBDR discipline: forward rendering, no Z-prepass, precise load/store ops, alpha-tested geometry in its own pipeline after solid opaque.

## Commands

Project not yet scaffolded. Once it is: `cargo test`, `cargo clippy`, `cargo run --release` (debug builds are too slow for meaningful play testing).
