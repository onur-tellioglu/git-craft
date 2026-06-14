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
