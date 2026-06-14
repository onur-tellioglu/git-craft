# git-craft — Design Specification

**Date:** 2026-06-11
**Status:** Approved for planning
**Target platform:** macOS Apple Silicon (developed and tuned on Apple M4, 10-core GPU, Metal 4, 16 GB unified memory, 3024×1964 display)

## 1. Overview

git-craft is a performance-focused Minecraft-style voxel game written in Rust on wgpu. Its differentiator is visual quality: the renderer targets the look of a modern Minecraft shader pack (BSL/Complementary class) out of the box — cascaded soft shadows, volumetric god rays, GTAO, bloom with ACES tonemapping, a physically based atmospheric sky, and screen-space reflective water.

The project doubles as a performance-engineering exercise: every system is built with measurement-first discipline (per-pass GPU timings, live engine statistics) and modern voxel-engine techniques (binary greedy meshing, vertex pulling, cave culling).

### Goals

- Playable creative-mode voxel sandbox: infinite world, block breaking/placing, walking and flying.
- Modern shader-pack visuals enabled by default.
- Performance target: **384-block render distance (24 vanilla-Minecraft chunks, i.e., a 12-column radius of git-craft's 32-block columns) at 120 fps** at native resolution on the M4, with all visual features enabled.
- Clean, testable engine core: meshing, lighting, worldgen, and physics are pure functions with unit tests.

### Non-Goals (v1)

- Survival mechanics: inventory screens, crafting, health/hunger, item drops.
- Mobs, NPCs, or any entities beyond the player.
- Multiplayer or networking.
- Audio.
- Cross-platform support beyond macOS (wgpu keeps the door open; nothing in the design blocks it, but it is not tested or tuned).
- Modding/scripting APIs.

## 2. Technology Stack

All versions verified mutually compatible as of 2026-06-11.

| Concern | Crate | Version |
|---|---|---|
| GPU API | wgpu | 29 |
| Windowing/input | winit | 0.30 |
| Math | glam | 0.33 |
| Byte casting | bytemuck | 1.25 |
| Async init blocking | pollster | 0.4 |
| Worker pool | rayon | 1.12 |
| Channels | crossbeam-channel | 0.5 |
| Noise | fastnoise-lite | 1.1 |
| Serialization | serde + postcard | 1 / 1.1 |
| Compression | lz4_flex | latest |
| Debug UI | egui + egui-wgpu + egui-winit | 0.34 |
| Image generation (build-time textures) | image | 0.25 |

Notable API constraints to code against:

- winit 0.30 mandates the `ApplicationHandler` pattern: the window is created inside `resumed()`, wrapped in `Arc<Window>`, and a clone is passed to `instance.create_surface()`. Rendering is driven by `WindowEvent::RedrawRequested` + `request_redraw()`.
- wgpu 29: `Surface::get_current_texture()` returns `CurrentSurfaceTexture`; `InstanceDescriptor` is replaced by explicit constructors; `depth_write_enabled`/`depth_compare` are `Option`.
- Metal present modes: `Fifo` and `Immediate` only (`Mailbox` panics). Use `AutoVsync`/`AutoNoVsync`.
- Surface format selected from `surface.get_capabilities()`, preferring the sRGB variant.
- Stay within the default limit of 4 bind groups for portability.
- Throttle the render loop on `WindowEvent::Occluded` (macOS does not block `get_current_texture()` for hidden windows).

## 3. Architecture

Single Rust binary crate.

```
git-craft/
├── Cargo.toml
├── build.rs                 # procedural texture generation at build time
├── assets/
│   └── shaders/             # WGSL files, loaded at runtime, hot-reloadable
└── src/
    ├── main.rs              # winit ApplicationHandler, main loop wiring
    ├── core/                # time step, config, helpers
    ├── world/               # block registry, chunk storage, worldgen, lighting, persistence
    ├── mesh/                # binary greedy meshing, quad packing
    ├── render/              # wgpu context, frame graph, pipelines, pass implementations
    ├── game/                # player controller, physics, block interaction, hotbar, day cycle
    └── debug/               # egui overlay: timings, counters, graphs
```

### Threading model

- **Main thread:** input → game logic → render, driven by `RedrawRequested`.
- **Rayon pool:** chunk generation and meshing jobs. Jobs are submitted individually via `rayon::spawn` (not bulk `par_iter`) so camera-near chunks can jump the queue; the submitter sorts pending requests by distance to camera each frame.
- **Crossbeam channel:** completed `(ChunkPos, MeshData)` / `(ChunkPos, ChunkData)` results flow back to the main thread, which drains the channel with a fixed per-frame GPU-upload budget to avoid frame spikes.
- **Persistence thread:** a low-priority background thread receives dirty chunk snapshots and writes region files; flushed on exit.

### Shader hot-reload

WGSL files in `assets/shaders/` are watched (mtime polling each frame is sufficient); on change, the affected pipeline is recompiled and swapped. Compile errors are printed and the old pipeline stays active. Shader iteration speed is a core requirement of this project.

## 4. World System

### Chunk storage

- The world is divided into **32³ block sections**; a vertical stack of 8 sections forms a column (world height 256). Horizontal extent is unbounded.
- Section data is **palette-compressed**: a per-section list of unique block IDs (the palette) plus a packed per-voxel index whose bit width grows with palette size. An all-air section stores no voxel data. Block IDs are `u16`.
- For meshing, a section is copied into a **34³ padded buffer** including a 1-voxel apron from the six neighboring sections, so face culling and ambient occlusion are seam-free across section borders.
- Loaded-chunk lifecycle: a ring of columns around the player is requested as the player moves; columns beyond render distance + margin are unloaded (saved first if dirty).

### World generation

Deterministic from a 64-bit seed, built on fastnoise-lite (OpenSimplex2 + FBM/ridged fractals).

- **Heightmap:** layered 2D noise — continentalness, erosion, and peaks combine into terrain height.
- **Biomes:** temperature + humidity noise fields select among six biomes: plains, forest, desert, mountains, snowy mountains, ocean. Biome blending interpolates surface block choice and tree density at borders.
- **Caves:** 3D noise — ridged "spaghetti" tunnels plus low-frequency "cheese" rooms, attenuated near the surface.
- **Water:** fixed sea level at y = 64.
- **Decoration:** trees (oak in plains/forest, spruce in snowy), cacti, flowers, tall grass, placed per-biome density. Structures that cross chunk borders use a pending-writes queue applied when the neighbor generates.
- **Block set (~16–20 types):** grass, dirt, stone, sand, snow-grass, water, oak log/leaves, spruce log/leaves, cactus, flower, tall grass, torch, plus a few decorative solids (planks, glass).

### Lighting (CPU flood-fill, Minecraft-style)

- Two 4-bit channels per voxel: **skylight** (seeded at 15 from the top of each column, propagated downward and outward) and **blocklight** (BFS from emitters such as torches).
- Block edits trigger incremental light updates (standard addition/removal BFS); every section whose light changed is queued for re-mesh.
- Light values are baked per-vertex into the mesh. The day/night cycle does NOT touch flood-fill data — darkness at night is applied in the shader via the sky color term (the same trick vanilla Minecraft uses).

### Persistence

- **Region files:** 32×32 columns per file, with an offset table header. Section payload is palette+indices serialized with postcard and compressed with lz4_flex.
- Only sections that diverge from worldgen output (player-modified) are written; everything else regenerates from the seed.
- Writes happen on the persistence thread; full flush on exit. Player position, look direction, hotbar selection, and time of day are stored in a separate small file.

## 5. Meshing

- **Binary greedy meshing** (the bitwise algorithm of cgerikj's `binary-greedy-meshing`, current state of the art): occupancy masks stored as one `u64` per column; face culling is `column & !(column >> 1)` per axis; merging uses AND/XOR sweeps, comparing block IDs only at merge time. Expected cost: 50–200 µs per section, making re-meshing effectively free at gameplay rates. Implemented in-repo (the crate serves as reference) because quad output must carry our packed attributes.
- **Quad packing — vertex pulling:** no vertex buffer. Each output quad is packed into **8 bytes (2×u32)** in a storage buffer: local position (6 bits/axis), face direction (3 bits), quad width/height (5–6 bits each), 4×2-bit corner AO, 4-bit skylight, 4-bit blocklight, texture-array layer (~10 bits). The vertex shader derives the corner from `vertex_index % 4`, unpacks with `extractBits()`, and computes tiled UVs from quad size (sampled with `fract()` against a texture array, so greedy quads tile correctly).
- **Per-vertex AO:** classic 0–3 corner rule (`side1 && side2 ? 0 : 3 − (side1+side2+corner)`) computed from the padded buffer, with the **quad diagonal flip** when `ao00+ao11 ≠ ao01+ao10` to remove the anisotropy artifact.
- **Mesh memory:** quads for all sections live in a few large **arena-allocated storage buffers** (slab allocator with free-list), so terrain renders from one bind group with one `draw_indexed_indirect` per visible section. Transparent geometry (water, leaves-as-translucent is NOT planned — leaves are alpha-tested) goes into a separate arena drawn in the transparent pass.

## 6. Rendering

### Strategy

**Single-pass forward rendering.** No deferred, no Z-prepass. Rationale: Apple TBDR GPUs eliminate opaque overdraw in hardware (making a Z-prepass redundant and deferred's main win moot), wgpu exposes no subpasses (so a G-buffer would burn memory bandwidth at pass boundaries), and a voxel game has exactly two analytic lights (sun/moon) with flood-fill covering all point lights — the many-lights problem deferred solves does not exist here.

TBDR discipline throughout: `LoadOp::Clear` wherever possible, `StoreOp::Discard` for anything not sampled later, few large passes rather than many small ones, compute work batched into contiguous encoders.

### Textures

- **2D texture array** (not an atlas): one 32×32 layer per block texture, with albedo, normal, and roughness arrays. No UV bleeding, mipmaps work, greedy quads tile via `fract()`.
- Textures are **procedurally generated** in `build.rs` (noise + per-block palettes) and embedded/loaded as array layers. git-craft owns its look; zero licensing concerns.

### Culling

- **CPU frustum culling:** AABB vs 6 planes over all loaded sections (~5k at full render distance: a 25×25 column grid × 8 vertical sections, <0.1 ms).
- **Cave culling (Tommaso Checchi visibility graph):** at mesh time, flood-fill the section's air voxels to compute a 15-bit face-to-face connectivity mask; per frame, BFS outward from the camera section, entering each neighbor only through connected faces. Culls 50–99% of underground geometry for microseconds of CPU.
- Survivors get their indirect-draw args written each frame. GPU occlusion culling (HZB) is explicitly deferred to a possible v2; multi-draw-indirect is unavailable on Metal through wgpu, and cave culling + hardware HSR already remove most overdraw.

### Frame graph

| # | Pass | Type | Output |
|---|---|---|---|
| 1 | Sky LUTs (Hillaire transmittance/multi-scatter once; sky-view + aerial-perspective per frame) + auto-exposure histogram | compute | small LUTs |
| 2 | Shadow cascades: 3 × 2048² depth-only CSM; far cascades updated every 2–4 frames; terrain drawn with a minimal vertex shader | render | depth ×3 |
| 3 | Opaque forward: full lighting in-shader; solid terrain first, then alpha-tested foliage as a separate pipeline (discard breaks HSR; isolate it) | render | HDR color (RGBA16F), depth (Store), normals+ambient-fraction (RGB10A2, Store) |
| 4 | GTAO half-res + blur; froxel volumetric scatter/integrate (samples CSM + flood-fill skylight, temporal jitter); water SSR half-res raymarch | compute | AO tex, froxel grid, SSR tex |
| 5 | Transparent: water (SSR + refraction from sampled opaque color, fallback to sky-view LUT on SSR miss) and particles (soft, depth-fade), sampling the froxel grid for fog | render | HDR color (depth read-only) |
| 6 | Composite & post: apply GTAO to ambient fraction, composite volumetrics, bloom (13-tap downsample chain from half-res + tent upsample), TAA, then final pass: auto-exposure, ACES tonemap, gamma | compute + render | swapchain |

### Lighting model (pass 3)

```
direct  = sunColor(timeOfDay) * NdotL * shadowCSM(pcf3x3)
ambient = skyColor * pow(skylightFF, curve) * vertexAO        // skylight doubles as cave light-leak guard
torch   = torchColor * pow(blocklightFF, curve) * vertexAO
color   = albedo * (direct * min(shadow, skylightGuard) + ambient + torch)
        + aerialPerspectiveFog
```

The flood-fill skylight term is multiplied into the direct term beyond shadow-map range and underground, guaranteeing dark caves where the CSM cannot reach.

### Anti-aliasing

**TAA** (not MSAA): GTAO and volumetrics already require temporal accumulation and jitter, and TAA is the natural shader-pack path. A render-scale option (default 1.0, fallback 0.75 with TAA upsampling) is the performance safety valve.

### GPU budget (estimated, native 3024×1964 on M4)

shadows ~1.2 ms + opaque ~2.5–3.0 ms + GTAO ~0.7 ms + volumetrics ~0.8 ms + SSR ~0.5 ms + transparents ~0.3 ms + bloom ~0.4 ms + sky/post ~0.4 ms ≈ **6.8–7.3 ms**, inside the 8.3 ms / 120 fps budget. wgpu timestamp queries measure every pass from day one; the estimate is validated continuously, not assumed.

## 7. Gameplay

### Player controller

- Two movement modes, toggled with double-space (or F): **walking** (gravity, jumping, swept axis-separated AABB collision against blocks — no corner snagging) and **flight** (creative-style, speed multiplier on sprint key).
- FPS camera from `DeviceEvent::MouseMotion` raw deltas with `CursorGrabMode::Locked`; Escape releases the cursor.
- Water: slow sinking, hold-jump to swim upward, reduced movement speed.

### Block interaction

- **DDA voxel raycast** (Amanatides & Woo), 6-block reach. Left click breaks instantly (creative); right click places the selected block against the hit face. Placement is rejected if the target cell intersects the player AABB.
- Targeted block gets a thin wireframe outline (small dedicated pipeline).
- Edit flow: block change → incremental light update → re-mesh affected sections (including neighbors when the edit touches a border or its apron).

### Hotbar and UI

- 9-slot hotbar bound to keys 1–9 and scroll wheel; shift+scroll pages through all placeable block types (creative: everything available).
- Center-screen crosshair.
- All UI (hotbar, crosshair, menus) rendered through egui — the debug overlay already requires it; a custom UI system is out of scope.

### Day/night cycle

- Full cycle in 20 minutes (configurable). Sun angle drives the Hillaire atmosphere (sunsets and dawn colors come out physically), the CSM light direction, and a weak bluish directional moon light at night.
- Flood-fill skylight values stay constant; night darkening happens entirely in the shader via the sky color term.

## 8. Performance Discipline & Debug HUD

- **egui debug HUD (F3):** FPS + frame-time graph, per-pass GPU times from timestamp queries, section counters (loaded / meshed / frustum-culled / cave-culled / drawn), draw call count, mesh arena occupancy, chunk generation and meshing queue depths, player position/biome.
- Every optimization claim must be validated against HUD numbers; "feels faster" does not count.
- A `--bench` flag runs a deterministic camera flight over a fixed seed and reports frame-time percentiles, enabling regression comparison between commits.

## 9. Testing Strategy

The engine core is pure functions over plain data — unit tested with TDD:

- **Meshing:** known voxel patterns → expected quad counts and geometry (single block = 6 quads; 2×2 flat surface = 1 merged quad; apron-neighbor face culling; AO corner values and diagonal flip).
- **Lighting:** BFS propagation correctness; incremental update after place/remove equals from-scratch recomputation.
- **Worldgen:** same seed → bit-identical chunk output (determinism).
- **Palette storage:** set/get roundtrip, palette growth/shrink.
- **Persistence:** save/load roundtrip equality; region file offset table integrity.
- **Physics:** AABB collision edge cases (corners, exact-touch, high velocity).
- **Shaders:** all WGSL compiles in a headless wgpu device test.

Rendering output is validated visually plus via HUD metrics and `--bench` percentiles.

## 10. Milestones

Each milestone ends in a working, committed, playable state.

- **M1 — Triangle to camera:** window, wgpu init, fly camera, one hardcoded test section rendered with packed quads. Foundation: ApplicationHandler, surface config, depth buffer, shader hot-reload.
- **M2 — World:** worldgen (biomes, caves, trees), chunk streaming around the camera, binary greedy meshing on rayon, arena buffers, frustum culling. Free flight through an infinite world.
- **M3 — Playable:** walking physics, block break/place with raycast and outline, hotbar, crosshair. The game exists.
- **M4 — Light:** flood-fill skylight + blocklight, torches, incremental updates, cave culling, day/night cycle driving a basic sun.
- **M5 — The shaders:** the ladder, one rung per commit — CSM shadows → Hillaire sky + aerial fog → bloom + ACES → GTAO + TAA → froxel volumetrics → water SSR + refraction.
- **M6 — Persistence & polish:** region save/load, texture polish (normal/roughness maps), performance pass validating 24 chunks @ 120 fps, `--bench` baseline.

## 11. Risks & Mitigations

- **GPU budget overrun at native res:** render-scale knob (0.75× + TAA upsample) ships in the options from M5; per-pass timestamps catch the offending pass early.
- **wgpu/Metal indirect-draw limitations:** design assumes one `draw_indexed_indirect` per visible section (~1–2k after culling), which Metal handles fine; no dependency on multi-draw-indirect.
- **Light update latency on large edits:** incremental BFS is bounded; worst case (removing a block under a tall column) is still local. If profiling shows spikes, light updates move to the rayon pool with a one-frame visual delay.
- **16 GB unified memory:** palette compression keeps 24-chunk world data in the low hundreds of MB; mesh arenas are capped with LRU eviction of distant sections.
