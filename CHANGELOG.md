# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the project is on `0.x`, breaking changes may occur between minor versions. The
minor version tracks roadmap milestone progress (e.g. `0.5` corresponds to milestone M5);
`1.0` lands once the planned milestones are complete.

## [Unreleased]

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
