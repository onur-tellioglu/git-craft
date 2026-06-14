# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the project is on `0.x`, breaking changes may occur between minor versions.

## [Unreleased]

## [0.1.1] - 2026-06-14

### Added

- Open-source release: dual MIT/Apache-2.0 license, README, VISION, CONTRIBUTING,
  CODE_OF_CONDUCT, SECURITY, AGENTS guide, issue/PR templates, and CI.

### Fixed

- Shader loading resolves `assets/shaders` relative to the executable, so a shipped
  binary with `assets/` bundled alongside runs outside the build tree. The compile-time
  `CARGO_MANIFEST_DIR` path stays the preferred dev/hot-reload location.

## [0.1.0] - 2026-06-14

### Added

- Voxel world: greedy-meshed chunks, infinite streaming worldgen, cave culling.
- Lighting: cascaded shadow maps (CSM), flood-fill block/sky light.
- Post-processing: GTAO, TAA, volumetric in-scatter, bloom, ACES tone mapping.
- Atmosphere: Hillaire sky and aerial-perspective LUTs, day/night cycle.
- Water: transparent refraction with screen-space reflections and a sky-LUT fallback.
- Performance tooling: F3 debug HUD with per-pass GPU timestamps, render-scale safety
  valve, `--bench` percentile reporting.
