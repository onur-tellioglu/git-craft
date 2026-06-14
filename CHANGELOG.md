# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the project is on `0.x`, breaking changes may occur between minor versions. The
minor version tracks roadmap milestone progress (e.g. `0.5` corresponds to milestone M5);
`1.0` lands once the planned milestones are complete.

## [Unreleased]

## [0.6.0] - 2026-06-14

Milestone M6: region persistence, procedural material textures, and a measurable --bench harness.

### Added

- `--bench` performance harness (M6a): a deterministic fixed-vantage flythrough at the
  full 384-block render distance, frozen at noon, that warms up until streaming goes
  idle, records a window of frames (default 600, `--bench-frames N` to override), and
  prints min/mean/p50/p95/p99/max for CPU frame time and GPU pass time with a PASS/FAIL
  verdict against the 120 fps (8.33 ms) budget. Uses `Immediate` present when the surface
  offers it so CPU frame time is not vsync-capped; the GPU metric (from the existing
  per-pass timestamps) is vsync-independent and drives the verdict. The `--bench` flag
  was previously documented but not wired up.
- M6 performance baseline (1280×720, render-scale 1.0): GPU p50 ≈ 13.3 ms / p99 ≈ 18.7 ms,
  CPU p50 ≈ 4.6 ms / p99 ≈ 8.9 ms. Verdict FAIL — the full M5 shader stack exceeds the
  native-resolution 120 fps budget; the render-scale safety valve and per-pass tuning are
  the follow-up performance work this baseline now measures against.
- Region save/load persistence (M6b): player edits now survive a restart. Broken/placed
  blocks are written to region files under `saves/region/` (32×32 columns per file,
  read-modify-written via a temp file + atomic rename) and reloaded when the player returns,
  instead of being overwritten by fresh worldgen. Only edited columns are persisted —
  untouched terrain regenerates deterministically — and lighting is never stored: a loaded
  column recomputes it through the generation path. All disk I/O runs on a dedicated worker
  thread, so the frame loop never blocks; edited columns are saved when they unload or on quit.
- Procedural per-block material textures (M6c): the flat per-block palette color is replaced
  by code-generated materials — per-block albedo detail, a tangent-space normal map, and a
  roughness channel — sampled in the terrain pass. Lighting gains normal-mapped surface relief
  and a roughness-controlled specular highlight. Materials are generated deterministically from
  each block's own base color (no external or proprietary art), with a full CPU-built mip chain
  so terrain tiling doesn't shimmer at distance.

### Fixed

- Corrupt region file (out-of-range packed palette indices) would panic the worker thread
  and silently discard all subsequent saves; `Section::read_bytes` now validates every
  packed index and returns `None` on violation so load fails cleanly to `Loaded::Failed`
  and the column regenerates instead.
- Save errors on column eviction were silently treated as success: the worker now sends
  `SaveOk`/`SaveFailed` acknowledgements over the result channel, and `saved_columns` is
  only updated once the worker confirms the write.
- Persistence worker is now disabled in `--bench` mode so benchmark flights do not write
  to `saves/region/` and skew reproducibility.
- Normal mapping no longer leaks diffuse/specular light onto geometrically back-facing
  surfaces (dark side of walls near the sun terminator was incorrectly lit). A geometric
  back-face guard (`ndotl_geo = dot(geo_normal, sun)`) now gates the entire direct +
  specular contribution so back faces receive only ambient light.

### Changed

- The device now requests `max_bind_groups = 5` (the terrain pipeline binds camera, quads,
  shadow, aerial, and the new material group); Apple Silicon Metal supports this comfortably.

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
