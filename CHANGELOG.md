# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the project is on `0.x`, breaking changes may occur between minor versions. The
minor version tracks roadmap milestone progress (e.g. `0.5` corresponds to milestone M5);
`1.0` lands once the planned milestones are complete.

## [Unreleased]

### Added

- Region save/load persistence (M6b): player edits now survive a restart. Broken/placed
  blocks are written to region files under `saves/region/` (32×32 columns per file,
  read-modify-written via a temp file + atomic rename) and reloaded when the player returns,
  instead of being overwritten by fresh worldgen. Only edited columns are persisted —
  untouched terrain regenerates deterministically — and lighting is never stored: a loaded
  column recomputes it through the generation path. All disk I/O runs on a dedicated worker
  thread, so the frame loop never blocks; edited columns are saved when they unload or on quit.

### Fixed

- Corrupt region file (out-of-range packed palette indices) would panic the worker thread
  and silently discard all subsequent saves; `Section::read_bytes` now validates every
  packed index and returns `None` on violation so load fails cleanly to `Loaded::Failed`
  and the column regenerates instead.
- Save errors on column eviction were silently treated as success: the worker now sends
  `SaveOk`/`SaveFailed` acknowledgements over the result channel, and `saved_columns` is
  only updated once the worker confirms the write.
- `Persistence` field is now `Option<Persistence>` so bench mode (M6a) can pass `None`
  without region files polluting benchmark reproducibility.

## [0.5.0] - 2026-06-14

First public release, at milestone M5 (the full shader ladder).

### Added

- Voxel world: greedy-meshed chunks, infinite streaming worldgen, cave culling.
- Lighting: cascaded shadow maps (CSM), flood-fill block/sky light.
- Post-processing: GTAO, TAA, volumetric in-scatter, bloom, ACES tone mapping.
- Atmosphere: Hillaire sky and aerial-perspective LUTs, day/night cycle.
- Water: transparent refraction with screen-space reflections and a sky-LUT fallback.
- Performance tooling: F3 debug HUD with per-pass GPU timestamps, render-scale safety
  valve, `--bench` percentile reporting.
- Open-source release: dual MIT/Apache-2.0 license, README, VISION, CONTRIBUTING,
  CODE_OF_CONDUCT, SECURITY, AGENTS guide, issue/PR templates, and CI.

### Fixed

- Shader loading resolves `assets/shaders` relative to the executable, so a shipped
  binary with `assets/` bundled alongside runs outside the build tree. The compile-time
  `CARGO_MANIFEST_DIR` path stays the preferred dev/hot-reload location.
