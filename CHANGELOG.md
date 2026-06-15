# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the project is on `0.x`, breaking changes may occur between minor versions. The
minor version tracks roadmap milestone progress (e.g. `0.5` corresponds to milestone M5);
`1.0` lands once the planned milestones are complete.

## [Unreleased]

## [0.6.3] - 2026-06-15

M6 review-minor cleanups: validate the region save-format version, deduplicate an internal byte-reader helper, and clarify bench/module documentation. No shader or visual behavior change.

### Changed

- **The region save format now validates its on-disk version field.** `parse_region`
  rejects (returns `None`) any region blob whose stored version does not match the
  current `VERSION`, instead of reading the field and silently discarding it. This
  guards against misreading a future, incompatible region layout as if it were the
  current one. Covered by a unit test that feeds a bumped-version blob and asserts
  rejection. (#11)

### Internal

- Deduplicated the `take<N>` fixed-size byte-reader helper into a single shared
  implementation in the `world` module; `region` and `section` now import it. No
  behavior change. (#11)
- Bench module documentation and test clarity: documented `WARMUP_CAP` semantics and
  `bench_yaw` edge cases, clarified the warmup yaw-freeze, and added an assertion for
  the last used frame index. (#10)
- Fixed stale doc comments in `block.rs`, `game_ui.rs`, and the M6c plan doc. The
  `terrain.wgsl` specular grazing-angle guard was reviewed, found not strictly
  redundant, and retained with an explanatory comment. (#12)

## [0.6.2] - 2026-06-15

Honest `--bench` verdict: switch PASS/FAIL to frame-coherent CPU frame-time p99 (the prior GPU-pass-timestamp sum overcounts ~3× on Apple TBDR). Add `--native-bench` for native-resolution measurement.

### Fixed

- **The `--bench` PASS/FAIL verdict now uses CPU frame-time p99 (frame-coherent),
  replacing the previous GPU-pass-timestamp-sum verdict.** On Apple TBDR (all
  Apple Silicon GPUs), consecutive render passes pipeline: pass N's fragment
  phase overlaps pass N+1's tiler phase. Summing per-pass timestamp deltas
  therefore overcounts true frame time — sometimes by roughly 3× (measured
  example: the F3 HUD showed 27.5 ms of summed GPU time while the game
  presented at 101 fps, i.e. 9.86 ms/frame actual). The old GPU-sum verdict
  consequently produced false FAILs: v0.6.1's "GPU p99 13.48 ms vs 8.33 ms"
  was an overcount, not a real budget miss.

  In `Immediate` present mode the CPU `dt` is frame-coherent and directly
  reflects uncapped render cost, making CPU p99 the reliable verdict metric.
  The GPU timestamp column is preserved in the bench report as a **diagnostic
  reference** — useful for identifying hotspot passes in relative terms —
  but it is clearly labelled as not driving the verdict and known to be
  incoherent across frames due to async readback latency (2+ frames under
  `max_frame_latency = 2`). A frame-coherent whole-frame GPU total is tracked
  in issue #18.

- `GpuTimer::total_ms()` now returns `max(end_ticks) − min(begin_ticks)` —
  the true whole-frame GPU wall-clock — instead of the sum of per-pass deltas.
  This value drives the F3 HUD GPU total and is available in the bench report
  diagnostic column. Per-pass `pass_ms` values in the HUD are unchanged
  (correct for identifying individual hotspot passes) but are **not additive
  on TBDR**; the GPU wall-clock is always less than or equal to their sum.

- Stale zero GPU samples in the bench (async readback not yet completed at the
  start of recording) are now filtered out of the GPU diagnostic distribution.
  These zeros were an artifact of the 2+ frame async readback lag, not real
  frame times.

### Added

- `--native-bench` flag: run the existing deterministic flythrough bench at the
  primary display's native physical pixel resolution (e.g. 3024×1964 on M4)
  instead of the hardcoded 1280×720. Requires `--bench`. The printed report now
  includes the resolution so multiple bench runs are distinguishable.
  This enables measuring the actual native-resolution GPU cost to track
  progress on the 120 fps / 8.33 ms budget goal (issue #13).

- `frame_wall_ms` helper (in `render/timestamps.rs`): computes the true
  whole-frame GPU wall-clock as `max(end_ticks) − min(begin_ticks)` across all
  render passes in a frame. Powers both `GpuTimer::total_ms()` (F3 HUD total)
  and the bench GPU diagnostic column. Zero-tick pairs (pass not issued) are
  excluded from the min/max reduction to avoid anchoring the wall-clock to an
  uninitialised slot. Note: this value is **not yet frame-coherent in the bench**
  under multi-frame readback latency (tracked in issue #18), so it does not
  drive the bench PASS/FAIL verdict.

## [0.6.1] - 2026-06-15

Reduce CSM shadow-cascade depth-raster and PCF shadow GPU cost at render-scale 1.0 (measured GPU p99 18.7 → 13.48 ms, −28%).

### Performance

- Reduced PCF shadow kernel in `terrain.wgsl` from 5×5 (25 taps) to 3×3 (9 taps),
  cutting per-fragment shadow evaluation cost by ~64%. Shadow penumbra softness is
  slightly reduced but remains visually acceptable at voxel scale.
- Reduced shadow-map resolution from 2048² to 1536² across all three cascades,
  cutting cascade depth-pass GPU cost by ~44% (area ratio 1536²/2048² ≈ 0.56).
  Normal-offset and slope-scale depth bias self-scale via `texel_world`; no acne
  introduced.
- 1280×720 render-scale 1.0 GPU benchmark — 600-frame deterministic flythrough,
  measured interactively with the window frontmost so TIMESTAMP_QUERY is active
  (before → after):
  GPU p50: 13.3 ms → 10.87 ms (−18%)
  GPU p99: 18.7 ms → 13.48 ms (−28%)
  After-run full GPU distribution: mean 11.01 ms, p95 12.37 ms, max 15.60 ms; CPU
  frame p50 4.32 ms / p99 9.40 ms. The measured reduction confirms the predicted
  ~64% fewer PCF taps + ~44% smaller shadow-depth raster area, with no shadow acne
  or aliasing regression (visually verified). Hitting the full 120 fps budget
  (GPU p99 ≤ 8.33 ms) remains the goal of issue #13 across further PRs.

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
