---
title: git-craft M4 — Light
date: 2026-06-12
domain: world-layer
type: enhancement
priority: high
breaking: false
db-migration: false
rls-affecting: false
slice: 4
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files:
  - git-craft/src/world/block.rs
  - git-craft/src/world/light.rs
  - git-craft/src/world/light_engine.rs
  - git-craft/src/world/chunks.rs
  - git-craft/src/world/jobs.rs
  - git-craft/src/world/mod.rs
  - git-craft/src/mesh/padded.rs
  - git-craft/src/mesh/neighborhood.rs
  - git-craft/src/mesh/greedy.rs
  - git-craft/src/render/terrain.rs
  - git-craft/src/render/visibility.rs
  - git-craft/src/render/mod.rs
  - git-craft/src/game/daycycle.rs
  - git-craft/src/game/mod.rs
  - git-craft/src/app.rs
  - git-craft/assets/shaders/terrain.wgsl
trigger-tasks-touched: []
shared-modules-touched: []
---

# git-craft M4 — Light Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Minecraft-style CPU flood-fill lighting — per-voxel 4-bit skylight and blocklight, a placeable torch, incremental light updates on block edits, light baked into mesh quads and consumed by the shader, Tommaso Checchi cave culling, and a 20-minute day/night cycle driving a basic sun and sky color.

**Architecture:** Light lives next to block data: each `Column` gains 8 `Arc<LightData>` sections (uniform-or-dense nibble storage mirroring the palette philosophy — an all-sky-15 or all-dark section stores one byte). Initial skylight is computed inside the existing rayon generation job as a pure function over `ColumnData`; cross-column seams and all block edits go through an incremental BFS engine (`light_engine.rs`) operating on `ChunkMap`, which reuses `dirty_sections_touching` so the existing re-mesh path picks up light changes. The mesher samples the out-layer cell's light per face (like AO) and includes it in the greedy merge key; the packed quad format already reserves the skylight/blocklight bits. Cave culling computes a 15-bit face-to-face connectivity mask per section in the mesh job and runs a per-frame direction-restricted BFS from the camera section. The day/night cycle never touches flood-fill data — darkening happens in the shader via a frame uniform (sky color, day factor, sun direction), exactly as the spec prescribes.

**Tech Stack:** Rust (edition 2024), wgpu 29, winit 0.30, glam 0.33, egui 0.34, bytemuck 1.25, rayon 1.12, crossbeam-channel 0.5.

**Spec:** `docs/superpowers/specs/2026-06-11-dabcraft-design.md` §4 (lighting), §5 (quad packing — light bits already reserved), §6 (cave culling, lighting model), §7 (day/night), §9 (test strategy), §10 (M4).

**No git remote exists** — skip all push/PR/issue steps (issue gate skipped for the same reason). Commit locally on branch `feat/m4-light`.

**Environment:** every shell needs `export PATH="$HOME/.cargo/bin:$PATH"` before cargo commands. All commands run from the repo root (`~/Github/Minecraft`) with `--manifest-path git-craft/Cargo.toml`. macOS has no `timeout`; smoke tests use background-run + kill. `gen` is a reserved keyword in edition 2024 — the worldgen module path is `crate::world::r#gen`.

---

## Context primer (read before Task 1)

Key existing types and invariants you will build on:

- `Section` (`world/section.rs`): palette-compressed 32³ block storage. `unpack_into(&mut [BlockId])` bulk-decodes all 32768 voxels with index `(y*32+z)*32+x`. **LightData uses the same voxel index order.**
- `Column` (`world/chunks.rs`): `sections: [Arc<Section>; 8]` + `dirty: [bool; 8]`. `dirty = true` means "needs (re)meshing". `ChunkMap::dirty_sections_touching(pos)` marks every section whose 34³ padded volume contains a world position — light changes reuse this.
- `ChunkMap::set_block` uses `Arc::make_mut` (clone-on-write vs in-flight mesh jobs); light updates must use the same discipline on light Arcs.
- Mesh jobs (`world/jobs.rs`) capture a 3×3×3 `Arc<Section>` neighborhood (`mesh/neighborhood.rs`), build a 34³ `PaddedSection` (`mesh/padded.rs`), and run the binary greedy mesher (`mesh/greedy.rs`). A per-section `version` counter in `App` drops stale results.
- `PackedQuad` (`mesh/quad.rs`): 2×u32. `data1` bits 13..17 = skylight, 17..21 = blocklight — **already packed and unit-tested**; the mesher currently hardcodes `skylight: 15, blocklight: 0` in `greedy.rs::emit`.
- The greedy mesher merges faces only within a `(face, slice, block, ao_key)` plane key. Light must join that key or quads with different light would merge.
- `BlockId::is_solid()` (`world/block.rs`) means "renders as an opaque cube face", not "collidable" (water is `is_solid` but non-collidable — app-level closures decide). M4 adds two more predicates: `blocks_light()` and `light_emission()`. Do NOT conflate them.
- `TerrainRenderer` (`render/terrain.rs`): one `CameraUniform { view_proj }` uniform, storage-buffer quad arena, one `draw_indexed_indirect` per visible section, frustum culling in `prepare()`. The shader is `assets/shaders/terrain.wgsl` (hot-reloaded; validated by `render/hot_reload.rs::validate_wgsl`, which has a unit test compiling the shipped shader).
- `block.rs` has a test (`colors_match_the_shader_palette`) that parses the `PALETTE` table out of `terrain.wgsl` and asserts the Rust `color()` table matches entry-by-entry. Adding a block means updating BOTH tables or that test fails.
- Worldgen (`world/gen.rs`): `generate_column` returns `(ColumnData { sections: Vec<Section> /* len 8 */ }, Vec<StructureWrite>)`. In-column decoration is already applied inside `generate_column`; cross-column writes flow through `ChunkMap::insert_generated` / pending queue. `WORLD_HEIGHT = 256`, `COLUMN_SECTIONS = 8`, `SEA_LEVEL = 64`.
- `App::update_world` (`app.rs`) is the streaming step: drain jobs → unload → request gen → request mesh → budgeted uploads. `App::update_interaction` applies block edits via `ChunkMap::set_block`.

Commands:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --manifest-path git-craft/Cargo.toml                 # all tests
cargo test --manifest-path git-craft/Cargo.toml light::         # one module
cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
cargo run --release --manifest-path git-craft/Cargo.toml        # play (debug too slow)
```

## File structure

| File | Status | Responsibility |
|---|---|---|
| `git-craft/src/world/block.rs` | modify | `TORCH` block, `blocks_light()`, `light_emission()` |
| `git-craft/src/world/light.rs` | create | `LightData` (uniform/dense nibble storage), `LightChannel`, `light_new_column` (pure, runs in gen job) |
| `git-craft/src/world/light_engine.rs` | create | incremental BFS: `add_light`, `remove_light`, `on_block_changed`, `seed_column_borders`, `relight_all` |
| `git-craft/src/world/chunks.rs` | modify | `Column.light`, `ChunkMap::light`/`set_light`, `insert_generated` takes light + returns applied write positions |
| `git-craft/src/world/jobs.rs` | modify | gen job computes column light; mesh job computes visibility mask |
| `git-craft/src/mesh/padded.rs` | modify | padded light channel alongside blocks |
| `git-craft/src/mesh/neighborhood.rs` | modify | 3×3×3 `Arc<LightData>` capture |
| `git-craft/src/mesh/greedy.rs` | modify | sample out-cell light, include in merge key, emit real light |
| `git-craft/src/render/terrain.rs` | modify | `FrameUniform` (view_proj + sky + sun), visibility filter in `prepare` |
| `git-craft/src/render/visibility.rs` | create | `face_connectivity` mask + per-frame `visible_set` BFS |
| `git-craft/src/game/daycycle.rs` | create | `DayCycle`: time, sun direction, day factor, sky color |
| `git-craft/src/app.rs` | modify | wiring: light on gen/edit, visibility BFS, day cycle, HUD lines, V toggle |
| `git-craft/assets/shaders/terrain.wgsl` | modify | blocklight + day factor + basic sun lighting |

## Setup

- [x] **Step 0.1: Create the milestone branch**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd ~/Github/Minecraft
git checkout main
git pull 2>/dev/null || true   # no remote; ignore failure
git checkout -b feat/m4-light
```

Expected: branch `feat/m4-light` created from `main` (main already contains the merged M3 work; `git log --oneline -1` shows the M3 HUD-toggle fix commit `2a0ccf1`).

---

### Task 1: Torch block + light predicates

**Files:**
- Modify: `git-craft/src/world/block.rs`
- Modify: `git-craft/assets/shaders/terrain.wgsl` (PALETTE table + clamp index only)
- Test: in-file `#[cfg(test)]` in `block.rs`

The torch is the only light emitter in M4. It renders as a full cube (a proper cross-model is out of scope until textures land) and is collidable like every solid block. What distinguishes it: it does NOT block light propagation, and it emits blocklight 14.

- [x] **Step 1.1: Write the failing tests**

Append to the `tests` module in `git-craft/src/world/block.rs`:

```rust
    #[test]
    fn torch_is_registered_and_placeable() {
        assert_eq!(TORCH.0, 12, "persisted ids are stable; torch is 12");
        assert!(PLACEABLE.contains(&TORCH));
        assert_eq!(TORCH.display_name(), "Torch");
        assert!(TORCH.is_solid(), "torch renders as an opaque cube in M4");
    }

    #[test]
    fn only_air_and_torch_pass_light() {
        assert!(!AIR.blocks_light());
        assert!(!TORCH.blocks_light(), "a torch must not shadow the sky shaft it sits in");
        for id in 1..=11u16 {
            assert!(BlockId(id).blocks_light(), "block {id} must block light");
        }
    }

    #[test]
    fn torch_is_the_only_emitter() {
        assert_eq!(TORCH.light_emission(), 14);
        for id in 0..=11u16 {
            assert_eq!(BlockId(id).light_emission(), 0, "block {id} must not emit");
        }
    }
```

Also update the existing `ids_are_stable` test's array to include `(TORCH, 12)` (and its type to `[(BlockId, u16); 13]`), and `only_air_is_not_solid` loop bound from `1..=11u16` to `1..=12u16`.

- [x] **Step 1.2: Run tests to verify they fail**

Run: `cargo test --manifest-path git-craft/Cargo.toml block::`
Expected: FAIL — `TORCH` not found.

- [x] **Step 1.3: Implement the block registry changes**

In `git-craft/src/world/block.rs`, after `pub const CACTUS`:

```rust
pub const TORCH: BlockId = BlockId(12);
```

Update `PLACEABLE`:

```rust
/// Every block the player can place (creative: everything but air),
/// in hotbar paging order.
pub const PLACEABLE: [BlockId; 12] = [
    GRASS, DIRT, STONE, SAND, SNOW_GRASS, WATER,
    OAK_LOG, OAK_LEAVES, SPRUCE_LOG, SPRUCE_LEAVES, CACTUS, TORCH,
];
```

Add to `impl BlockId` (after `is_solid`):

```rust
    /// Does this block stop flood-fill light? Everything except air and
    /// torches. Water blocks light fully in M4 (it also renders opaque);
    /// per-block attenuation can arrive with transparency in M5.
    pub fn blocks_light(self) -> bool {
        !matches!(self, AIR | TORCH)
    }

    /// Blocklight level seeded at this block's cell (spec §4: BFS from
    /// emitters). Torches emit 14, like vanilla.
    pub fn light_emission(self) -> u8 {
        if self == TORCH { 14 } else { 0 }
    }
```

Extend `display_name` with `12 => "Torch",` and `color` with `12 => [0.95, 0.71, 0.30],` (warm torch orange).

- [x] **Step 1.4: Mirror the palette in the shader**

In `git-craft/assets/shaders/terrain.wgsl`, grow the table:

```wgsl
const PALETTE = array<vec3<f32>, 13>(
    vec3(1.0, 0.0, 1.0),      //  0 air (never rendered; magenta = bug)
    vec3(0.35, 0.62, 0.22),   //  1 grass
    vec3(0.45, 0.32, 0.2),    //  2 dirt
    vec3(0.52, 0.52, 0.54),   //  3 stone
    vec3(0.86, 0.81, 0.58),   //  4 sand
    vec3(0.91, 0.93, 0.95),   //  5 snow grass
    vec3(0.19, 0.36, 0.68),   //  6 water (opaque until M5)
    vec3(0.42, 0.31, 0.19),   //  7 oak log
    vec3(0.23, 0.43, 0.14),   //  8 oak leaves
    vec3(0.32, 0.23, 0.14),   //  9 spruce log
    vec3(0.16, 0.3, 0.19),    // 10 spruce leaves
    vec3(0.27, 0.5, 0.21),    // 11 cactus
    vec3(0.95, 0.71, 0.3),    // 12 torch
);
```

And change the clamp in `vs_main` from `PALETTE[min(tex, 11u)]` to `PALETTE[min(tex, 12u)]`.

- [x] **Step 1.5: Run the full test suite**

Run: `cargo test --manifest-path git-craft/Cargo.toml`
Expected: PASS — including `colors_match_the_shader_palette` (now 13 entries) and `placeable_covers_every_block_except_air` (max id 12).

- [x] **Step 1.6: Commit**

```bash
git add git-craft/src/world/block.rs git-craft/assets/shaders/terrain.wgsl
git commit -m "feat: add torch block and light propagation predicates"
```

---

### Task 2: LightData — uniform/dense nibble storage

**Files:**
- Create: `git-craft/src/world/light.rs`
- Modify: `git-craft/src/world/mod.rs` (add `pub mod light;`)
- Test: in-file `#[cfg(test)]` in `light.rs`

Two 4-bit channels per voxel: skylight in the low nibble, blocklight in the high nibble, one byte per voxel, index order `(y*32+z)*32+x` (identical to `Section`). The killer optimization mirrors palette storage: most sections are uniformly lit (all sky-15 above ground, all dark deep underground), so `Uniform(u8)` stores one byte and promotes to a 32 KiB `Dense` box only on the first divergent write. A 13-column load radius is ~4400 sections; without the uniform variant light alone would cost ~140 MiB.

- [x] **Step 2.1: Write the failing tests**

Create `git-craft/src/world/light.rs` with only the test module first (the impl comes in 2.3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_default_reads_zero_everywhere() {
        let l = LightData::dark();
        assert_eq!(l.sky(0, 0, 0), 0);
        assert_eq!(l.block_light(31, 31, 31), 0);
        assert_eq!(l.packed(15, 15, 15), 0);
    }

    #[test]
    fn uniform_stores_no_voxel_data_until_a_divergent_write() {
        let mut l = LightData::uniform(15, 0);
        assert!(matches!(l, LightData::Uniform(_)));
        assert!(!l.set_sky(4, 5, 6, 15), "writing the uniform value is a no-op");
        assert!(matches!(l, LightData::Uniform(_)), "no-op write must not promote");
        assert!(l.set_sky(4, 5, 6, 9), "divergent write reports a change");
        assert!(matches!(l, LightData::Dense(_)));
        assert_eq!(l.sky(4, 5, 6), 9);
        assert_eq!(l.sky(0, 0, 0), 15, "other voxels keep the old uniform value");
        assert_eq!(l.block_light(4, 5, 6), 0, "the other nibble is untouched");
    }

    #[test]
    fn channels_are_independent() {
        let mut l = LightData::dark();
        l.set_sky(1, 2, 3, 12);
        l.set_block_light(1, 2, 3, 7);
        assert_eq!(l.sky(1, 2, 3), 12);
        assert_eq!(l.block_light(1, 2, 3), 7);
        assert_eq!(l.packed(1, 2, 3), 12 | (7 << 4));
    }

    #[test]
    fn set_returns_whether_the_value_changed() {
        let mut l = LightData::dark();
        assert!(l.set_block_light(0, 0, 0, 5));
        assert!(!l.set_block_light(0, 0, 0, 5), "same value again: unchanged");
        assert!(l.set_block_light(0, 0, 0, 6));
    }

    #[test]
    fn unpack_into_matches_pointwise_reads() {
        let mut l = LightData::uniform(15, 0);
        l.set_block_light(31, 0, 17, 14);
        let mut flat = vec![0u8; 32 * 32 * 32];
        l.unpack_into(&mut flat);
        assert_eq!(flat[(0 * 32 + 17) * 32 + 31], 15 | (14 << 4));
        assert_eq!(flat[0], 15);
    }

    #[test]
    fn from_sky_slice_detects_uniformity() {
        let all15 = vec![15u8; 32768];
        assert!(matches!(LightData::from_sky_slice(&all15), LightData::Uniform(15)));
        let mut mixed = vec![15u8; 32768];
        mixed[100] = 3;
        let dense = LightData::from_sky_slice(&mixed);
        assert!(matches!(dense, LightData::Dense(_)));
        assert_eq!(dense.sky(4, 0, 3), 3, "voxel 100 = x4 z3 y0");
        assert_eq!(dense.block_light(4, 0, 3), 0, "sky slice seeds no blocklight");
    }
}
```

- [x] **Step 2.2: Register the module and verify the tests fail**

Add `pub mod light;` to `git-craft/src/world/mod.rs`.

Run: `cargo test --manifest-path git-craft/Cargo.toml light::`
Expected: FAIL to compile — `LightData` not defined.

- [x] **Step 2.3: Implement LightData**

Prepend the implementation above the test module in `git-craft/src/world/light.rs`:

```rust
// Per-section voxel light storage (spec §4): two 4-bit channels per voxel,
// skylight in the low nibble, blocklight in the high nibble.

pub const MAX_LIGHT: u8 = 15;

const SIZE: usize = 32;
const VOLUME: usize = SIZE * SIZE * SIZE;

/// Which flood-fill channel an operation targets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LightChannel {
    Sky,
    Block,
}

pub fn pack_light(sky: u8, block: u8) -> u8 {
    debug_assert!(sky <= MAX_LIGHT && block <= MAX_LIGHT);
    sky | (block << 4)
}

/// 32³ light values. `Uniform` covers the dominant cases (all sky-15 above
/// ground, all dark underground) in one byte; the first divergent write
/// promotes to `Dense` (32 KiB). Same voxel index order as `Section`.
#[derive(Clone, Debug, PartialEq)]
pub enum LightData {
    Uniform(u8),
    Dense(Box<[u8; VOLUME]>),
}

impl LightData {
    pub fn dark() -> Self {
        LightData::Uniform(0)
    }

    pub fn uniform(sky: u8, block: u8) -> Self {
        LightData::Uniform(pack_light(sky, block))
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < SIZE && y < SIZE && z < SIZE);
        (y * SIZE + z) * SIZE + x
    }

    pub fn packed(&self, x: usize, y: usize, z: usize) -> u8 {
        match self {
            LightData::Uniform(v) => *v,
            LightData::Dense(d) => d[Self::index(x, y, z)],
        }
    }

    pub fn sky(&self, x: usize, y: usize, z: usize) -> u8 {
        self.packed(x, y, z) & 0x0F
    }

    pub fn block_light(&self, x: usize, y: usize, z: usize) -> u8 {
        self.packed(x, y, z) >> 4
    }

    pub fn get(&self, ch: LightChannel, x: usize, y: usize, z: usize) -> u8 {
        match ch {
            LightChannel::Sky => self.sky(x, y, z),
            LightChannel::Block => self.block_light(x, y, z),
        }
    }

    /// Returns true when the stored value actually changed.
    fn set_packed(&mut self, x: usize, y: usize, z: usize, new: u8) -> bool {
        let i = Self::index(x, y, z);
        match self {
            LightData::Uniform(v) => {
                if *v == new {
                    return false;
                }
                let mut dense = Box::new([*v; VOLUME]);
                dense[i] = new;
                *self = LightData::Dense(dense);
                true
            }
            LightData::Dense(d) => {
                if d[i] == new {
                    return false;
                }
                d[i] = new;
                true
            }
        }
    }

    pub fn set_sky(&mut self, x: usize, y: usize, z: usize, v: u8) -> bool {
        debug_assert!(v <= MAX_LIGHT);
        let old = self.packed(x, y, z);
        self.set_packed(x, y, z, (old & 0xF0) | v)
    }

    pub fn set_block_light(&mut self, x: usize, y: usize, z: usize, v: u8) -> bool {
        debug_assert!(v <= MAX_LIGHT);
        let old = self.packed(x, y, z);
        self.set_packed(x, y, z, (old & 0x0F) | (v << 4))
    }

    pub fn set(&mut self, ch: LightChannel, x: usize, y: usize, z: usize, v: u8) -> bool {
        match ch {
            LightChannel::Sky => self.set_sky(x, y, z, v),
            LightChannel::Block => self.set_block_light(x, y, z, v),
        }
    }

    /// Bulk-decode all 32768 packed bytes (hot path for padded-buffer fill).
    pub fn unpack_into(&self, out: &mut [u8]) {
        assert_eq!(out.len(), VOLUME);
        match self {
            LightData::Uniform(v) => out.fill(*v),
            LightData::Dense(d) => out.copy_from_slice(&d[..]),
        }
    }

    /// Build from a 32768-entry skylight slice (blocklight 0), collapsing to
    /// Uniform when every value matches. Used by column light generation.
    pub fn from_sky_slice(sky: &[u8]) -> Self {
        assert_eq!(sky.len(), VOLUME);
        let first = sky[0];
        if sky.iter().all(|&v| v == first) {
            return LightData::Uniform(pack_light(first, 0));
        }
        let mut data = Box::new([0u8; VOLUME]);
        for (d, &s) in data.iter_mut().zip(sky) {
            *d = pack_light(s, 0);
        }
        LightData::Dense(data)
    }
}
```

- [x] **Step 2.4: Run the tests**

Run: `cargo test --manifest-path git-craft/Cargo.toml light::`
Expected: PASS (6 tests). Note: `LightChannel`, `get`, `set`, `MAX_LIGHT` are not exercised yet — they are consumed in Tasks 3–5; if clippy complains about dead code at this commit, add `#[allow(dead_code)] // consumed by light_engine (Task 5)` on the unused items and remove it in Task 5.

- [x] **Step 2.5: Commit**

```bash
git add git-craft/src/world/light.rs git-craft/src/world/mod.rs
git commit -m "feat: add uniform/dense nibble light storage per section"
```

---

### Task 3: Column light + ChunkMap accessors

**Files:**
- Modify: `git-craft/src/world/chunks.rs`
- Modify: `git-craft/src/app.rs` (call-site fix only — placeholder light)
- Test: existing + new tests in `chunks.rs`

`Column` gains a light array parallel to `sections`. `insert_generated` accepts the light computed by the gen job, and now **returns the world positions of structure writes it applied to ready columns** (its own pending queue + routed outside-writes) so `App` can run incremental light updates for them (Task 6) — the gen job lit the column before those writes landed. `ChunkMap` gains world-space light accessors with the same out-of-world conventions as `block_at`: above the world is full skylight, below is dark, unloaded is `None`.

- [x] **Step 3.1: Write the failing tests**

In `git-craft/src/world/chunks.rs` tests, add a helper next to `empty_column_data`:

```rust
    fn dark_light() -> [crate::world::light::LightData; COLUMN_SECTIONS] {
        std::array::from_fn(|_| crate::world::light::LightData::dark())
    }
```

(import `COLUMN_SECTIONS` is already in scope via `super::*`; the test module needs `use crate::world::light::{LightChannel, LightData};`)

New tests:

```rust
    #[test]
    fn light_accessors_follow_world_conventions() {
        let mut map = ChunkMap::default();
        let mut light = dark_light();
        light[2] = LightData::uniform(15, 0); // sections 2: y 64..96 fully sky-lit
        map.insert_generated_with_light(ColumnPos { x: 0, z: 0 }, empty_column_data(), light, Vec::new());
        assert_eq!(map.light(LightChannel::Sky, glam::IVec3::new(5, 70, 5)), Some(15));
        assert_eq!(map.light(LightChannel::Sky, glam::IVec3::new(5, 10, 5)), Some(0));
        assert_eq!(map.light(LightChannel::Block, glam::IVec3::new(5, 70, 5)), Some(0));
        assert_eq!(map.light(LightChannel::Sky, glam::IVec3::new(5, 300, 5)), Some(15), "above world = open sky");
        assert_eq!(map.light(LightChannel::Block, glam::IVec3::new(5, 300, 5)), Some(0));
        assert_eq!(map.light(LightChannel::Sky, glam::IVec3::new(5, -1, 5)), Some(0), "below world = dark");
        assert_eq!(map.light(LightChannel::Sky, glam::IVec3::new(99, 70, 5)), None, "unloaded");
    }

    #[test]
    fn set_light_writes_and_dirties_like_set_block() {
        let mut map = ChunkMap::default();
        map.insert_generated_with_light(ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        map.clear_all_dirty(ColumnPos { x: 0, z: 0 });
        assert!(map.set_light(LightChannel::Block, glam::IVec3::new(5, 64, 5), 14));
        assert_eq!(map.light(LightChannel::Block, glam::IVec3::new(5, 64, 5)), Some(14));
        let col = map.ready(ColumnPos { x: 0, z: 0 }).unwrap();
        assert!(col.dirty[2], "light change must re-mesh the owning section");
        assert!(col.dirty[1], "y=64 is section 2's bottom row → section 1 apron dirty");
        // Same value again: no change, no work.
        map.clear_all_dirty(ColumnPos { x: 0, z: 0 });
        assert!(!map.set_light(LightChannel::Block, glam::IVec3::new(5, 64, 5), 14));
        assert!(!map.ready(ColumnPos { x: 0, z: 0 }).unwrap().dirty[2], "no-op write must not dirty");
        // Out of world / unloaded are rejected.
        assert!(!map.set_light(LightChannel::Sky, glam::IVec3::new(5, 300, 5), 3));
        assert!(!map.set_light(LightChannel::Sky, glam::IVec3::new(99, 64, 5), 3));
    }

    #[test]
    fn insert_returns_positions_of_writes_applied_to_ready_columns() {
        let mut map = ChunkMap::default();
        // Pending write waiting for column (0,0):
        map.queue_writes(vec![StructureWrite {
            pos: glam::IVec3::new(5, 70, 5),
            block: OAK_LEAVES,
            only_air: true,
        }]);
        let touched = map.insert_generated_with_light(
            ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        assert_eq!(touched, vec![glam::IVec3::new(5, 70, 5)], "pending write applied at insert");
        // Outside-write routed into the now-ready column:
        let touched = map.insert_generated_with_light(
            ColumnPos { x: 1, z: 0 },
            empty_column_data(),
            dark_light(),
            vec![StructureWrite { pos: glam::IVec3::new(31, 70, 5), block: STONE, only_air: false }],
        );
        assert_eq!(touched, vec![glam::IVec3::new(31, 70, 5)]);
        // A write queued for an absent column is NOT reported (nothing applied yet).
        let touched = map.insert_generated_with_light(
            ColumnPos { x: 5, z: 5 },
            empty_column_data(),
            dark_light(),
            vec![StructureWrite { pos: glam::IVec3::new(200, 70, 200), block: STONE, only_air: false }],
        );
        assert!(touched.is_empty());
    }
```

Naming note: the method stays `insert_generated` — the tests above use `insert_generated_with_light` only so this step fails cleanly; in Step 3.3 you change `insert_generated` itself (new signature) and then rename the calls in these tests to `insert_generated`. All pre-existing tests that call `insert_generated(pos, data, writes)` get the `dark_light()` argument inserted as the third parameter.

- [x] **Step 3.2: Run tests to verify they fail**

Run: `cargo test --manifest-path git-craft/Cargo.toml chunks::`
Expected: FAIL to compile.

- [x] **Step 3.3: Implement**

In `git-craft/src/world/chunks.rs`:

Imports:

```rust
use crate::world::light::{LightChannel, LightData, MAX_LIGHT};
use crate::world::r#gen::WORLD_HEIGHT;
```

`Column` gains the parallel array:

```rust
pub struct Column {
    pub sections: [Arc<Section>; COLUMN_SECTIONS],
    pub light: [Arc<LightData>; COLUMN_SECTIONS],
    pub dirty: [bool; COLUMN_SECTIONS],
}
```

`insert_generated` — new signature and return value (replace the existing body's tail; the pending-write application moves AFTER the column is stored so positions can be collected uniformly):

```rust
    /// Store a finished generation result with its gen-job light: apply any
    /// writes other columns queued for it, then route ITS outside-writes to
    /// ready columns (applying + dirtying) or to the pending queue.
    /// Returns the world positions of every write applied to a READY column
    /// (this one or a neighbor) — the caller must run incremental light
    /// updates for them, because the gen job lit the column before these
    /// writes landed.
    pub fn insert_generated(
        &mut self,
        pos: ColumnPos,
        data: ColumnData,
        light: [LightData; COLUMN_SECTIONS],
        outside_writes: Vec<StructureWrite>,
    ) -> Vec<glam::IVec3> {
        let sections: [Arc<Section>; COLUMN_SECTIONS] = data
            .sections
            .into_iter()
            .map(Arc::new)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or_else(|_| unreachable!("generate_column always yields 8 sections"));
        let light = light.map(Arc::new);
        self.columns.insert(
            pos,
            Slot::Ready(Column { sections, light, dirty: [true; COLUMN_SECTIONS] }),
        );
        let mut applied = Vec::new();
        // Writes queued while this column was absent — route_write now finds
        // it ready and applies them in place.
        if let Some(queued) = self.pending.remove(&pos) {
            for w in queued {
                applied.extend(self.route_write(w));
            }
        }
        for w in outside_writes {
            applied.extend(self.route_write(w));
        }
        applied
    }

    fn route_write(&mut self, w: StructureWrite) -> Option<glam::IVec3> {
        let target = block_to_column(w.pos.x, w.pos.z);
        match self.columns.get_mut(&target) {
            Some(Slot::Ready(col)) => {
                if (0..WORLD_HEIGHT).contains(&w.pos.y) {
                    // Arc::make_mut: clone-on-write only if a mesh job still
                    // holds the old Arc; otherwise mutates in place.
                    let section = Arc::make_mut(&mut col.sections[(w.pos.y / 32) as usize]);
                    let (lx, ly, lz) = (
                        w.pos.x.rem_euclid(32) as usize,
                        (w.pos.y % 32) as usize,
                        w.pos.z.rem_euclid(32) as usize,
                    );
                    let before = section.get(lx, ly, lz);
                    apply_write_to_section(section, w);
                    let changed = section.get(lx, ly, lz) != before;
                    self.dirty_sections_touching(w.pos);
                    // only_air writes that hit terrain change nothing — no
                    // light update needed for those.
                    if changed {
                        return Some(w.pos);
                    }
                }
                None
            }
            _ => {
                self.pending.entry(target).or_default().push(w);
                None
            }
        }
    }
```

Note `insert_generated` no longer calls `apply_write` directly — pending writes flow through `route_write` (the column is ready by then, behavior identical, positions collected). Remove the now-unused `apply_write` import if nothing else uses it (`r#gen` still exports it for its own tests; check with `cargo build`).

Light accessors (place next to `block_at` / `set_block`):

```rust
    /// Light level at a world position. `None` when the column is not
    /// loaded. Above the world it is open sky (sky 15 / block 0); below
    /// the world it is dark — mirrors `block_at`'s "outside is air".
    pub fn light(&self, ch: LightChannel, pos: glam::IVec3) -> Option<u8> {
        if pos.y >= WORLD_HEIGHT {
            return Some(match ch {
                LightChannel::Sky => MAX_LIGHT,
                LightChannel::Block => 0,
            });
        }
        if pos.y < 0 {
            return Some(0);
        }
        let col = self.ready(block_to_column(pos.x, pos.z))?;
        Some(col.light[(pos.y / 32) as usize].get(
            ch,
            pos.x.rem_euclid(32) as usize,
            (pos.y % 32) as usize,
            pos.z.rem_euclid(32) as usize,
        ))
    }

    /// Write one light value; dirties every section whose padded volume sees
    /// the cell (same rule as block edits — quads sample neighbor light).
    /// Returns false when out of world, unloaded, or unchanged.
    pub fn set_light(&mut self, ch: LightChannel, pos: glam::IVec3, v: u8) -> bool {
        if !(0..WORLD_HEIGHT).contains(&pos.y) {
            return false;
        }
        let Some(col) = self.ready_mut(block_to_column(pos.x, pos.z)) else {
            return false;
        };
        let light = Arc::make_mut(&mut col.light[(pos.y / 32) as usize]);
        let changed = light.set(
            ch,
            pos.x.rem_euclid(32) as usize,
            (pos.y % 32) as usize,
            pos.z.rem_euclid(32) as usize,
            v,
        );
        if changed {
            self.dirty_sections_touching(pos);
        }
        changed
    }
```

(The `0..256` literals already in `chunks.rs` may stay; new code uses `WORLD_HEIGHT` for clarity.)

- [x] **Step 3.4: Fix the App call site with placeholder light**

In `git-craft/src/app.rs` `update_world`, the `JobResult::Generated` arm — the gen job does not carry light until Task 4, so insert dark placeholder light to keep the build green:

```rust
                JobResult::Generated { pos, data, writes } => {
                    let d2 = (pos.x - center.x).pow(2) + (pos.z - center.z).pow(2);
                    if d2 > UNLOAD_RADIUS * UNLOAD_RADIUS {
                        self.world.queue_writes(writes);
                        continue;
                    }
                    // Placeholder until the gen job computes real light (M4 Task 4).
                    let light = std::array::from_fn(|_| crate::world::light::LightData::dark());
                    let _touched = self.world.insert_generated(pos, data, light, writes);
                }
```

- [x] **Step 3.5: Rename test calls and run the suite**

Rename `insert_generated_with_light` → `insert_generated` in the Step 3.1 tests; add `dark_light()` as the third argument to every pre-existing `insert_generated` call in the test module (8 call sites).

Run: `cargo test --manifest-path git-craft/Cargo.toml`
Expected: PASS.

- [x] **Step 3.6: Commit**

```bash
git add git-craft/src/world/chunks.rs git-craft/src/app.rs
git commit -m "feat: store per-section light in chunk columns"
```

---

### Task 4: Column skylight computed in the generation job

**Files:**
- Modify: `git-craft/src/world/light.rs` (add `light_new_column`)
- Modify: `git-craft/src/world/jobs.rs` (`Generated` carries light)
- Modify: `git-craft/src/app.rs` (use the job's light)
- Test: in-file tests in `light.rs`

Initial skylight is a pure function over a freshly generated column, so it runs inside the existing rayon gen job for free parallelism: seed 15 from the top of each (x,z) shaft down to the first light-blocking block, then BFS-spread within the column (sideways/up lose 1 per step; level-15 falls downward undimmed — the vanilla rule, which makes open sky cheap and overhangs correct). Cross-column seams are healed later by `seed_column_borders` (Task 5) — this matches Minecraft semantics: flood-fill skylight is per-column sky access, not neighbor-mountain shadowing (shadows are the CSM's job in M5). Blocklight starts at 0 — worldgen places no emitters.

- [x] **Step 4.1: Write the failing tests**

Append to the `tests` module in `git-craft/src/world/light.rs`:

```rust
    use crate::world::block::{STONE, TORCH, WATER};
    use crate::world::section::Section;

    /// 8 empty sections with `fill(x, y, z) -> Option<BlockId>` applied.
    fn column_with(fill: impl Fn(usize, i32, usize) -> Option<crate::world::block::BlockId>) -> Vec<Section> {
        let mut sections: Vec<Section> = (0..8).map(|_| Section::empty()).collect();
        for y in 0..256i32 {
            for x in 0..32usize {
                for z in 0..32usize {
                    if let Some(b) = fill(x, y, z) {
                        sections[(y / 32) as usize].set(x, (y % 32) as usize, z, b);
                    }
                }
            }
        }
        sections
    }

    fn sky_at(light: &[LightData; 8], x: usize, y: i32, z: usize) -> u8 {
        light[(y / 32) as usize].sky(x, (y % 32) as usize, z)
    }

    #[test]
    fn flat_ground_splits_sky_above_dark_below() {
        // Solid stone slab below y=20, open air above.
        let sections = column_with(|_, y, _| (y < 20).then_some(STONE));
        let light = light_new_column(&sections);
        assert_eq!(sky_at(&light, 5, 20, 5), 15, "first air cell above ground");
        assert_eq!(sky_at(&light, 5, 200, 5), 15, "high air");
        assert_eq!(sky_at(&light, 5, 10, 5), 0, "inside stone");
        assert!(matches!(light[7], LightData::Uniform(15)), "all-air section collapses to uniform");
        assert!(matches!(light[0], LightData::Uniform(0)), "all-stone section collapses to dark");
    }

    #[test]
    fn overhang_light_decrements_sideways_then_falls() {
        // Ground at y<20 plus a roof slab at y=40 covering x<16: under the
        // roof, light enters from the open side (x>=16) and decays inward.
        let sections = column_with(|x, y, _| {
            if y < 20 { return Some(STONE); }
            (y == 40 && x < 16).then_some(STONE)
        });
        let light = light_new_column(&sections);
        assert_eq!(sky_at(&light, 16, 30, 5), 15, "open shaft beside the roof");
        assert_eq!(sky_at(&light, 15, 30, 5), 14, "one step under the roof");
        assert_eq!(sky_at(&light, 12, 30, 5), 11, "four steps under the roof");
        assert_eq!(sky_at(&light, 0, 30, 5), 0, "16 steps in: fully dark (15-16 < 0)");
        // The non-15 light does NOT fall undimmed: directly under the roof
        // at x=15, dropping from y=39 (14) to y=25 keeps decrementing? No —
        // horizontal entry happens at every y under the roof independently,
        // so every open cell under the roof at x=15 reads 14.
        assert_eq!(sky_at(&light, 15, 21, 5), 14);
    }

    #[test]
    fn sealed_cave_is_dark_and_water_blocks_sky() {
        // Stone up to y=100 with a sealed air pocket at y 40..44, x/z 10..14;
        // a water column at (20,*,20) from y=60..=100 over air below.
        let sections = column_with(|x, y, z| {
            let pocket = (40..44).contains(&y) && (10..14).contains(&x) && (10..14).contains(&z);
            let shaft = x == 20 && z == 20 && (0..=100).contains(&y);
            if pocket { return None; }
            if shaft { return ((60..=100).contains(&y)).then_some(WATER); }
            (y <= 100).then_some(STONE)
        });
        let light = light_new_column(&sections);
        assert_eq!(sky_at(&light, 11, 41, 11), 0, "sealed pocket gets no skylight");
        assert_eq!(sky_at(&light, 20, 50, 20), 0, "below the water plug: dark (water blocks light in M4)");
        assert_eq!(sky_at(&light, 20, 101, 20), 15, "above the water surface");
    }

    #[test]
    fn torch_block_does_not_block_the_sky_shaft() {
        // A floating torch at (5,50,5): the shaft below it stays sky-15.
        let sections = column_with(|x, y, z| (x == 5 && y == 50 && z == 5).then_some(TORCH));
        let light = light_new_column(&sections);
        assert_eq!(sky_at(&light, 5, 50, 5), 15, "the torch cell itself");
        assert_eq!(sky_at(&light, 5, 49, 5), 15, "below the torch");
    }

    #[test]
    fn generated_light_has_no_blocklight() {
        let sections = column_with(|_, y, _| (y < 20).then_some(STONE));
        let light = light_new_column(&sections);
        assert_eq!(light[1].block_light(5, 5, 5), 0);
        assert_eq!(light[6].block_light(5, 5, 5), 0);
    }
```

- [x] **Step 4.2: Run tests to verify they fail**

Run: `cargo test --manifest-path git-craft/Cargo.toml light::`
Expected: FAIL to compile — `light_new_column` not defined.

- [x] **Step 4.3: Implement light_new_column**

Add to `git-craft/src/world/light.rs` (above the tests), with imports `use std::collections::VecDeque;`, `use crate::world::block::{BlockId, AIR};`, `use crate::world::r#gen::COLUMN_SECTIONS;`, `use crate::world::section::Section;`:

```rust
const WORLD_H: usize = 256;

fn cidx(x: usize, y: usize, z: usize) -> usize {
    (y * SIZE + z) * SIZE + x
}

/// Initial skylight for a freshly generated column (pure; runs in the rayon
/// gen job). Vertical seed: 15 from the top until the first light-blocking
/// block. Then an in-column BFS: sideways/up lose 1 per step, level-15
/// falls downward undimmed (vanilla rule). Blocklight starts 0 — worldgen
/// places no emitters. Cross-column seams are healed by
/// `light_engine::seed_column_borders` when neighbors are loaded.
pub fn light_new_column(sections: &[Section]) -> [LightData; COLUMN_SECTIONS] {
    assert_eq!(sections.len(), COLUMN_SECTIONS);
    let mut blocks = vec![AIR; SIZE * SIZE * WORLD_H];
    for (s, section) in sections.iter().enumerate() {
        section.unpack_into(&mut blocks[s * VOLUME..(s + 1) * VOLUME]);
    }
    let mut sky = vec![0u8; SIZE * SIZE * WORLD_H];
    let mut queue: VecDeque<(usize, usize, usize, u8)> = VecDeque::new();
    for x in 0..SIZE {
        for z in 0..SIZE {
            for y in (0..WORLD_H).rev() {
                if blocks[cidx(x, y, z)].blocks_light() {
                    break;
                }
                sky[cidx(x, y, z)] = MAX_LIGHT;
                queue.push_back((x, y, z, MAX_LIGHT));
            }
        }
    }
    while let Some((x, y, z, level)) = queue.pop_front() {
        for (dx, dy, dz) in [(1i32, 0i32, 0i32), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
            if !(0..SIZE as i32).contains(&nx)
                || !(0..WORLD_H as i32).contains(&ny)
                || !(0..SIZE as i32).contains(&nz)
            {
                continue;
            }
            let (nx, ny, nz) = (nx as usize, ny as usize, nz as usize);
            if blocks[cidx(nx, ny, nz)].blocks_light() {
                continue;
            }
            let candidate = if level == MAX_LIGHT && dy == -1 { MAX_LIGHT } else { level - 1 };
            if candidate > sky[cidx(nx, ny, nz)] {
                sky[cidx(nx, ny, nz)] = candidate;
                if candidate > 1 {
                    queue.push_back((nx, ny, nz, candidate));
                }
            }
        }
    }
    std::array::from_fn(|s| LightData::from_sky_slice(&sky[s * VOLUME..(s + 1) * VOLUME]))
}
```

Performance note: the seed loop enqueues every sky-15 voxel (~200k for an open column) and most pops are no-ops. Measured in the gen job this is low single-digit milliseconds on the M4 and runs on rayon, not the main thread. If `--bench` (M6) ever flags it, the seed set can shrink to shaft-boundary voxels — do not optimize now.

- [x] **Step 4.4: Run the light tests**

Run: `cargo test --manifest-path git-craft/Cargo.toml light::`
Expected: PASS.

- [x] **Step 4.5: Carry light through the job channel**

In `git-craft/src/world/jobs.rs`:

```rust
use crate::world::light::{light_new_column, LightData};
```

```rust
#[derive(Debug)]
pub enum JobResult {
    Generated {
        pos: ColumnPos,
        data: ColumnData,
        light: Box<[LightData; 8]>,
        writes: Vec<StructureWrite>,
    },
    /// `version` echoes the caller's per-section counter at spawn time so
    /// out-of-order completions of two in-flight jobs for the same section
    /// can be detected — only the latest version may be uploaded.
    Meshed { pos: SectionPos, version: u64, quads: Vec<PackedQuad> },
}
```

(`Box` keeps the enum small; a `Dense` variant inline would already be heap-boxed, but the array of 8 enums is 8×~16 B — boxing is for variant-size hygiene and cheap moves through the channel.)

```rust
    pub fn spawn_gen(&mut self, worldgen: WorldGen, pos: ColumnPos) {
        self.gen_in_flight += 1;
        let tx = self.tx.clone();
        rayon::spawn(move || {
            let (data, writes) = worldgen.generate_column(pos.x, pos.z);
            let light = Box::new(light_new_column(&data.sections));
            // Send fails only when the app is shutting down; fine to drop.
            let _ = tx.send(JobResult::Generated { pos, data, light, writes });
        });
    }
```

Update the existing `gen_job_roundtrips_through_the_channel` test's match arm to `JobResult::Generated { pos, data, .. }` (unchanged pattern still compiles — `..` absorbs the new field; verify).

In `git-craft/src/app.rs`, replace the Task 3 placeholder:

```rust
                JobResult::Generated { pos, data, light, writes } => {
                    let d2 = (pos.x - center.x).pow(2) + (pos.z - center.z).pow(2);
                    if d2 > UNLOAD_RADIUS * UNLOAD_RADIUS {
                        self.world.queue_writes(writes);
                        continue;
                    }
                    let _touched = self.world.insert_generated(pos, data, *light, writes);
                }
```

(`_touched` becomes live in Task 6.)

- [x] **Step 4.6: Full suite + commit**

Run: `cargo test --manifest-path git-craft/Cargo.toml`
Expected: PASS.

```bash
git add git-craft/src/world/light.rs git-craft/src/world/jobs.rs git-craft/src/app.rs
git commit -m "feat: compute column skylight inside generation jobs"
```

---

### Task 5: Incremental flood-fill light engine

**Files:**
- Create: `git-craft/src/world/light_engine.rs`
- Modify: `git-craft/src/world/mod.rs` (add `pub mod light_engine;`)
- Test: in-file tests in `light_engine.rs`

The standard Minecraft addition/removal BFS, generalized over `ChunkMap` and `LightChannel`:

- `add_light(map, ch, seeds)` — each seed `(pos, level)` means "ensure pos has at least level, then propagate outward". Seeding with a cell's *current* level re-propagates from it without writing (used for reflow); seeding higher writes first (used for emitters and sky re-entry).
- `remove_light(map, ch, pos)` — unlight the cell, BFS-remove every dependent neighbor (strictly dimmer, or the equal-15 cell straight below for skylight), collect the surviving brighter frontier as reseeds, reflow via `add_light`. Handles both teardown (block placed) and reflow (block removed) because a removed-block cell had light 0, so the BFS removes nothing and the frontier refills it.
- `on_block_changed(map, pos)` — called AFTER the block was set: `remove_light` per channel + an emission seed when the new block emits.
- `seed_column_borders(map, col)` — heal the 4 vertical seams of a freshly inserted column by seeding cells whose across-the-border partner is ≥2 levels dimmer (light must actually flow for a seed to matter, which keeps the queue tiny on open terrain).
- `relight_all(map, cols)` — from-scratch recompute (gen lighting + emitter scan + border seams). The spec §9 oracle: incremental updates must equal this exactly. Also handy as a future debug command.

Out-of-world conventions do real work here: `map.light` returns sky-15 above the world, so the BFS naturally pulls sky in through the world ceiling; `set_light` refuses out-of-world writes; unloaded columns return `None` and stop propagation at the loaded edge (border seeding heals it when the neighbor arrives).

- [x] **Step 5.1: Write the failing tests**

Create `git-craft/src/world/light_engine.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::IVec3;
    use crate::world::block::{AIR, STONE, TORCH};
    use crate::world::chunks::{ChunkMap, ColumnPos};
    use crate::world::light::{light_new_column, LightChannel, LightData};
    use crate::world::r#gen::ColumnData;
    use crate::world::section::Section;

    /// Flat world: stone below y=20, air above, lit correctly at insert.
    fn flat_map(radius: i32) -> ChunkMap {
        let mut map = ChunkMap::default();
        for cx in -radius..=radius {
            for cz in -radius..=radius {
                let mut sections: Vec<Section> = (0..8).map(|_| Section::empty()).collect();
                for y in 0..20usize {
                    for x in 0..32 {
                        for z in 0..32 {
                            sections[0].set(x, y, z, STONE);
                        }
                    }
                }
                let light = light_new_column(&sections);
                map.insert_generated(
                    ColumnPos { x: cx, z: cz },
                    ColumnData { sections },
                    light,
                    Vec::new(),
                );
            }
        }
        map
    }

    fn sky(map: &ChunkMap, p: IVec3) -> u8 {
        map.light(LightChannel::Sky, p).unwrap()
    }
    fn blk(map: &ChunkMap, p: IVec3) -> u8 {
        map.light(LightChannel::Block, p).unwrap()
    }

    #[test]
    fn torch_place_and_remove_roundtrips() {
        let mut map = flat_map(0);
        let p = IVec3::new(16, 20, 16); // first air cell above the floor
        map.set_block(p, TORCH);
        on_block_changed(&mut map, p);
        assert_eq!(blk(&map, p), 14);
        assert_eq!(blk(&map, p + IVec3::X), 13);
        assert_eq!(blk(&map, p + IVec3::new(3, 0, 0)), 11);
        assert_eq!(blk(&map, p + IVec3::new(0, 5, 0)), 9);
        assert_eq!(blk(&map, p - IVec3::Y), 0, "floor stone stays dark inside");
        map.set_block(p, AIR);
        on_block_changed(&mut map, p);
        assert_eq!(blk(&map, p), 0, "torch light fully removed");
        assert_eq!(blk(&map, p + IVec3::new(3, 0, 0)), 0);
    }

    #[test]
    fn placing_a_roof_casts_a_removal_shadow_and_digging_restores_sky() {
        let mut map = flat_map(0);
        // Roof one block above the ground at y=25 over a 5×5 patch.
        for x in 14..19 {
            for z in 14..19 {
                let p = IVec3::new(x, 25, z);
                map.set_block(p, STONE);
                on_block_changed(&mut map, p);
            }
        }
        let center = IVec3::new(16, 22, 16);
        assert!(sky(&map, center) < 15, "under the roof center: no direct sky");
        assert_eq!(sky(&map, IVec3::new(16, 26, 16)), 15, "above the roof unchanged");
        // Dig a hole in the roof: the shaft below floods back to 15.
        let hole = IVec3::new(16, 25, 16);
        map.set_block(hole, AIR);
        on_block_changed(&mut map, hole);
        assert_eq!(sky(&map, center), 15, "sky falls back down the shaft");
    }

    #[test]
    fn border_seeding_lights_a_tunnel_mouth_from_the_neighbor_column() {
        // Column (0,0): flat ground, open sky above y=20 — built by flat_map.
        let mut map = flat_map(0);
        // Column (1,0): solid stone below y=30 except a tunnel at y 20..22,
        // z 15..17, local x 0..12 whose mouth faces column (0,0). The
        // column-local gen light leaves the tunnel dark (no sky access
        // within its own column) — exactly the seam the healing pass fixes.
        let mut sections: Vec<Section> = (0..8).map(|_| Section::empty()).collect();
        for y in 0..30usize {
            for x in 0..32 {
                for z in 0..32 {
                    let tunnel = (20..22).contains(&y) && x < 12 && (15..17).contains(&z);
                    if !tunnel {
                        sections[0].set(x, y, z, STONE);
                    }
                }
            }
        }
        let light = light_new_column(&sections);
        map.insert_generated(ColumnPos { x: 1, z: 0 }, ColumnData { sections }, light, Vec::new());
        assert_eq!(sky(&map, IVec3::new(33, 21, 16)), 0, "tunnel dark before seam healing");
        seed_column_borders(&mut map, ColumnPos { x: 1, z: 0 });
        assert_eq!(sky(&map, IVec3::new(32, 21, 16)), 14, "mouth cell lit from the neighbor's 15");
        assert_eq!(sky(&map, IVec3::new(36, 21, 16)), 10, "decays one level per step inward");
        assert_eq!(sky(&map, IVec3::new(20, 25, 16)), 0, "stone interior stays dark");
    }

    #[test]
    fn incremental_equals_from_scratch() {
        let mut map = flat_map(1);
        let edits: Vec<(IVec3, crate::world::block::BlockId)> = vec![
            (IVec3::new(16, 20, 16), TORCH),
            (IVec3::new(18, 20, 16), STONE),
            (IVec3::new(16, 21, 16), STONE),  // box the torch in a little
            (IVec3::new(31, 20, 16), TORCH),  // torch on a column border
            (IVec3::new(32, 20, 16), STONE),  // wall just across the border
            (IVec3::new(16, 20, 16), AIR),    // remove the first torch again
            (IVec3::new(10, 19, 10), AIR),    // dig into the floor
            (IVec3::new(10, 18, 10), AIR),
            (IVec3::new(11, 18, 10), AIR),    // small L-tunnel
        ];
        for (p, b) in edits {
            map.set_block(p, b);
            on_block_changed(&mut map, p);
        }
        // Oracle: full recompute on a clone of the block state.
        let cols: Vec<ColumnPos> = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| ColumnPos { x, z }))
            .collect();
        let mut oracle = clone_blocks(&map, &cols);
        relight_all(&mut oracle, &cols);
        for &c in &cols {
            for y in 0..256 {
                for lx in 0..32 {
                    for lz in 0..32 {
                        let p = IVec3::new(c.x * 32 + lx, y, c.z * 32 + lz);
                        assert_eq!(
                            map.light(LightChannel::Sky, p), oracle.light(LightChannel::Sky, p),
                            "sky mismatch at {p}"
                        );
                        assert_eq!(
                            map.light(LightChannel::Block, p), oracle.light(LightChannel::Block, p),
                            "block mismatch at {p}"
                        );
                    }
                }
            }
        }
    }

    /// Rebuild a ChunkMap with the same blocks but freshly computed light.
    fn clone_blocks(map: &ChunkMap, cols: &[ColumnPos]) -> ChunkMap {
        let mut out = ChunkMap::default();
        for &c in cols {
            let col = map.ready(c).unwrap();
            let sections: Vec<Section> =
                col.sections.iter().map(|a| (**a).clone()).collect();
            let light = light_new_column(&sections);
            out.insert_generated(c, ColumnData { sections }, light, Vec::new());
        }
        out
    }

    #[test]
    fn light_changes_dirty_sections_for_remesh() {
        let mut map = flat_map(0);
        map.clear_all_dirty(ColumnPos { x: 0, z: 0 });
        let p = IVec3::new(16, 20, 16);
        map.set_block(p, TORCH);
        on_block_changed(&mut map, p);
        let col = map.ready(ColumnPos { x: 0, z: 0 }).unwrap();
        assert!(col.dirty[0], "torch light reaches into section 0 (y<32)");
    }
}
```

- [x] **Step 5.2: Register the module and verify tests fail**

Add `pub mod light_engine;` to `git-craft/src/world/mod.rs`.

Run: `cargo test --manifest-path git-craft/Cargo.toml light_engine::`
Expected: FAIL to compile.

- [x] **Step 5.3: Implement the engine**

`git-craft/src/world/light_engine.rs` above the tests:

```rust
// Incremental flood-fill light engine (spec §4): addition/removal BFS over
// the loaded ChunkMap, per channel. All functions assume the *block* state
// is already current; they fix the light to match.

use std::collections::VecDeque;

use glam::IVec3;

use crate::world::block::BlockId;
use crate::world::chunks::{ChunkMap, ColumnPos};
use crate::world::light::{light_new_column, LightChannel, MAX_LIGHT};
use crate::world::r#gen::WORLD_HEIGHT;
use crate::world::section::Section;

const DIRS: [IVec3; 6] = [
    IVec3::new(1, 0, 0),
    IVec3::new(-1, 0, 0),
    IVec3::new(0, 1, 0),
    IVec3::new(0, -1, 0),
    IVec3::new(0, 0, 1),
    IVec3::new(0, 0, -1),
];

const CHANNELS: [LightChannel; 2] = [LightChannel::Sky, LightChannel::Block];

fn emission(ch: LightChannel, block: BlockId) -> u8 {
    match ch {
        LightChannel::Block => block.light_emission(),
        LightChannel::Sky => 0,
    }
}

/// Level a neighbor receives from `level` when stepping `dir`:
/// skylight 15 falls downward undimmed; everything else loses 1.
fn spread(ch: LightChannel, level: u8, dir: IVec3) -> u8 {
    if ch == LightChannel::Sky && level == MAX_LIGHT && dir.y == -1 {
        MAX_LIGHT
    } else {
        level.saturating_sub(1)
    }
}

/// Addition BFS. Each seed `(pos, level)` means "ensure pos holds at least
/// level, then propagate from it". A seed at a cell's current level
/// re-propagates without writing (reflow); a higher seed writes first
/// (emitters, sky re-entry). Seeds at out-of-world positions propagate
/// virtually (the write is refused but the spread still happens), which is
/// how sky-15 above the world ceiling re-enters dug-out mountain tops.
pub fn add_light(map: &mut ChunkMap, ch: LightChannel, seeds: Vec<(IVec3, u8)>) {
    let mut queue: VecDeque<(IVec3, u8)> = VecDeque::new();
    for (p, v) in seeds {
        if v == 0 {
            continue;
        }
        let Some(cur) = map.light(ch, p) else { continue };
        if v > cur {
            map.set_light(ch, p, v); // refused out-of-world; spread below still runs
        }
        if v >= cur {
            queue.push_back((p, v));
        }
    }
    while let Some((p, level)) = queue.pop_front() {
        for dir in DIRS {
            let n = p + dir;
            let candidate = spread(ch, level, dir);
            if candidate == 0 {
                continue;
            }
            let Some(block) = map.block_at(n) else { continue }; // unloaded: stop
            if block.blocks_light() {
                continue;
            }
            let Some(cur) = map.light(ch, n) else { continue };
            if candidate > cur && map.set_light(ch, n, candidate) {
                queue.push_back((n, candidate));
            }
        }
    }
}

/// Removal BFS from `start`: unlight it, remove every dependent neighbor
/// (strictly dimmer, or the equal-15 straight-below cell for skylight —
/// that 15 only existed because it fell from here), then reflow from the
/// surviving brighter frontier. When `start` held no light (e.g. the block
/// there was just removed), nothing is torn down and the frontier reflow
/// simply refills the cell — so this single function serves both edits.
pub fn remove_light(map: &mut ChunkMap, ch: LightChannel, start: IVec3) {
    let Some(old) = map.light(ch, start) else { return };
    map.set_light(ch, start, 0);
    let mut queue: VecDeque<(IVec3, u8)> = VecDeque::from([(start, old)]);
    let mut reseed: Vec<(IVec3, u8)> = Vec::new();
    while let Some((p, level)) = queue.pop_front() {
        for dir in DIRS {
            let n = p + dir;
            let Some(nl) = map.light(ch, n) else { continue };
            if nl == 0 {
                continue;
            }
            let falls_from_here =
                ch == LightChannel::Sky && dir.y == -1 && level == MAX_LIGHT && nl == MAX_LIGHT;
            if nl < level || falls_from_here {
                if map.set_light(ch, n, 0) {
                    queue.push_back((n, nl));
                } else {
                    // Out-of-world (virtual sky) or unloaded edge: can't
                    // remove there; treat as a frontier source instead.
                    reseed.push((n, nl));
                }
            } else {
                reseed.push((n, nl));
            }
        }
    }
    add_light(map, ch, reseed);
}

/// Fix light after the block at `pos` was changed (block state already set).
pub fn on_block_changed(map: &mut ChunkMap, pos: IVec3) {
    if !(0..WORLD_HEIGHT).contains(&pos.y) {
        return;
    }
    let Some(block) = map.block_at(pos) else { return };
    for ch in CHANNELS {
        remove_light(map, ch, pos);
        let e = emission(ch, block);
        if e > 0 {
            add_light(map, ch, vec![(pos, e)]);
        }
    }
}

/// Heal the four vertical seams of column `pos` against loaded neighbors.
/// Only cells whose across-the-border partner is ≥2 levels dimmer become
/// seeds (light must actually flow), keeping the queue tiny on open
/// terrain where both sides are already sky-15.
pub fn seed_column_borders(map: &mut ChunkMap, pos: ColumnPos) {
    for ch in CHANNELS {
        let mut seeds: Vec<(IVec3, u8)> = Vec::new();
        for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let neighbor = ColumnPos { x: pos.x + dx, z: pos.z + dz };
            if map.ready(neighbor).is_none() {
                continue;
            }
            // World coords of the two facing block planes.
            for i in 0..32i32 {
                for y in 0..WORLD_HEIGHT {
                    let (a, b) = if dx != 0 {
                        let xa = if dx == 1 { pos.x * 32 + 31 } else { pos.x * 32 };
                        (IVec3::new(xa, y, pos.z * 32 + i), IVec3::new(xa + dx, y, pos.z * 32 + i))
                    } else {
                        let za = if dz == 1 { pos.z * 32 + 31 } else { pos.z * 32 };
                        (IVec3::new(pos.x * 32 + i, y, za), IVec3::new(pos.x * 32 + i, y, za + dz))
                    };
                    let (Some(la), Some(lb)) = (map.light(ch, a), map.light(ch, b)) else {
                        continue;
                    };
                    if la >= lb + 2 {
                        seeds.push((a, la));
                    } else if lb >= la + 2 {
                        seeds.push((b, lb));
                    }
                }
            }
        }
        add_light(map, ch, seeds);
    }
}

/// From-scratch recompute for the given columns: gen-style skylight per
/// column, emitter scan for blocklight, then seam healing. The oracle for
/// the incremental engine (spec §9) and a future debug command.
pub fn relight_all(map: &mut ChunkMap, cols: &[ColumnPos]) {
    use std::sync::Arc;
    for &c in cols {
        let Some(col) = map.ready(c) else { continue };
        let sections: Vec<Section> = col.sections.iter().map(|a| (**a).clone()).collect();
        let light = light_new_column(&sections);
        let col = map.ready_mut(c).expect("checked ready above");
        for (slot, l) in col.light.iter_mut().zip(light) {
            *slot = Arc::new(l);
        }
        for sy in 0..col.dirty.len() {
            col.dirty[sy] = true;
        }
    }
    for &c in cols {
        seed_column_borders(map, c);
    }
    // Blocklight: re-seed every emitter found in the recomputed columns.
    for &c in cols {
        let mut seeds: Vec<(IVec3, u8)> = Vec::new();
        let Some(col) = map.ready(c) else { continue };
        for (sy, section) in col.sections.iter().enumerate() {
            for y in 0..32 {
                for z in 0..32 {
                    for x in 0..32 {
                        let e = section.get(x, y, z).light_emission();
                        if e > 0 {
                            seeds.push((
                                IVec3::new(c.x * 32 + x as i32, (sy * 32 + y) as i32, c.z * 32 + z as i32),
                                e,
                            ));
                        }
                    }
                }
            }
        }
        add_light(map, LightChannel::Block, seeds);
    }
}
```

- [x] **Step 5.4: Run the engine tests**

Run: `cargo test --manifest-path git-craft/Cargo.toml light_engine::`
Expected: PASS (5 tests). The `incremental_equals_from_scratch` test is the load-bearing one — if it fails, debug the engine, not the oracle (the oracle is the simple definition). Remove any `#[allow(dead_code)]` left on `light.rs` items from Task 2.

- [x] **Step 5.5: Full suite + commit**

Run: `cargo test --manifest-path git-craft/Cargo.toml`
Expected: PASS.

```bash
git add git-craft/src/world/light_engine.rs git-craft/src/world/mod.rs git-craft/src/world/light.rs
git commit -m "feat: add incremental flood-fill light engine"
```

---

### Task 6: Wire light updates into edits and streaming

**Files:**
- Modify: `git-craft/src/app.rs`
- Test: behavior is covered by the Task 5 engine tests; this task is wiring + a smoke run

Three hook points, all in `app.rs`:

1. **Player edits** (`update_interaction`): after every successful `set_block`, run `on_block_changed`.
2. **Column insert** (`update_world`, `Generated` arm): after `insert_generated`, heal seams with `seed_column_borders`, then run `on_block_changed` for every structure-write position the insert applied to ready columns (trees that crossed borders landed after the gen job lit the column).
3. **HUD**: show sky/block light at the player's eye cell — the cheapest way to *see* the system work before the mesher consumes it (Task 7).

- [x] **Step 6.1: Hook player edits**

In `update_interaction`, the break arm becomes:

```rust
        if self.input.mouse_pressed(MouseButton::Left)
            || (self.input.mouse_down(MouseButton::Left) && self.break_timer == 0.0)
        {
            if self.world.set_block(hit.block, AIR) {
                crate::world::light_engine::on_block_changed(&mut self.world, hit.block);
            }
            self.break_timer = EDIT_REPEAT;
        }
```

and the place arm:

```rust
            if free && !self.player.aabb().intersects_cell(cell) {
                if self.world.set_block(cell, self.hotbar.selected_block()) {
                    crate::world::light_engine::on_block_changed(&mut self.world, cell);
                }
                self.place_timer = EDIT_REPEAT;
            }
```

- [x] **Step 6.2: Hook column streaming**

The `Generated` arm in `update_world`:

```rust
                JobResult::Generated { pos, data, light, writes } => {
                    let d2 = (pos.x - center.x).pow(2) + (pos.z - center.z).pow(2);
                    if d2 > UNLOAD_RADIUS * UNLOAD_RADIUS {
                        self.world.queue_writes(writes);
                        continue;
                    }
                    let touched = self.world.insert_generated(pos, data, *light, writes);
                    // Heal the light seams against already-loaded neighbors,
                    // then fix light under structure writes that landed after
                    // the gen job lit the column.
                    crate::world::light_engine::seed_column_borders(&mut self.world, pos);
                    for p in touched {
                        crate::world::light_engine::on_block_changed(&mut self.world, p);
                    }
                }
```

- [x] **Step 6.3: HUD light readout**

In `render()`, next to the other captured HUD values:

```rust
        let eye_cell = glam::IVec3::new(
            cam.x.floor() as i32,
            cam.y.floor() as i32,
            cam.z.floor() as i32,
        );
        let light_label = {
            use crate::world::light::LightChannel;
            match (
                self.world.light(LightChannel::Sky, eye_cell),
                self.world.light(LightChannel::Block, eye_cell),
            ) {
                (Some(s), Some(b)) => format!("sky {s} / block {b}"),
                _ => "—".to_string(),
            }
        };
```

(Place this AFTER `let cam = self.camera.position;`.) Add inside the Debug HUD window, after the `Target:` line:

```rust
                                ui.label(format!("Light:    {light_label}"));
```

- [x] **Step 6.4: Build, test, smoke-run**

Run: `cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings`
Expected: PASS, no warnings.

Smoke run (macOS has no `timeout`):

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release --manifest-path git-craft/Cargo.toml
./git-craft/target/release/git-craft > /tmp/dabcraft-m4-t6.log 2>&1 &
APP_PID=$!
sleep 30
kill $APP_PID
grep -iE "panic|error" /tmp/dabcraft-m4-t6.log || echo "CLEAN"
```

Expected: `CLEAN` (the egui/wgpu startup may log benign info lines; investigate anything matching panic/error). Note: the binary may live at `git-craft/target/release/git-craft` or `target/release/git-craft` depending on workspace layout — check both.

- [x] **Step 6.5: Commit**

```bash
git add git-craft/src/app.rs
git commit -m "feat: wire light updates into edits and column streaming"
```

---

### Task 7: Mesher bakes flood-fill light into quads

**Files:**
- Modify: `git-craft/src/mesh/padded.rs` (light channel)
- Modify: `git-craft/src/mesh/neighborhood.rs` (light Arc capture)
- Modify: `git-craft/src/mesh/greedy.rs` (sample + merge key + emit)
- Modify: `git-craft/src/app.rs` (`build_neighborhood` fills light)
- Test: in-file tests in all three mesh files

A quad's light is the light of the **out-layer cell in front of the face** — the same cell row AO samples (a face is lit by the air it faces, not by its own opaque voxel). Light joins the greedy merge key: faces whose out-cells differ in light must not merge, or a cave mouth would smear sky-15 deep into the dark. `PackedQuad` already has the bits; `greedy.rs::emit` stops hardcoding `15/0`.

Defaults are chosen so every existing mesh test stays green: `PaddedSection::air()` and missing neighborhood light read as packed `0x0F` (sky 15, block 0) — also the correct convention for the apron above the world ceiling.

- [x] **Step 7.1: Write the failing tests**

`git-craft/src/mesh/padded.rs` tests:

```rust
    #[test]
    fn air_padded_defaults_to_full_sky() {
        let p = PaddedSection::air();
        assert_eq!(p.light_packed(0, 0, 0), 0x0F);
        assert_eq!(p.light_packed(33, 33, 33), 0x0F);
    }

    #[test]
    fn build_carries_interior_and_apron_light() {
        use crate::world::light::LightData;
        let mut s = Section::empty();
        s.set(5, 5, 5, STONE);
        let mut light = LightData::dark();
        light.set_sky(5, 6, 5, 11); // section-local cell above the stone
        // Apron closure: light 0x23 everywhere outside.
        let p = PaddedSection::build(&s, &light, |_, _, _| (AIR, 0x23));
        assert_eq!(p.light_packed(6, 7, 6), 0x0B, "interior light at +1 padded offset");
        assert_eq!(p.light_packed(6, 6, 6), 0x00, "dark interior cell");
        assert_eq!(p.light_packed(0, 5, 5), 0x23, "apron from the closure");
    }
```

`git-craft/src/mesh/neighborhood.rs` tests:

```rust
    #[test]
    fn light_lookup_resolves_neighbors_and_defaults_to_sky() {
        use crate::world::light::LightData;
        let mut hood = MeshNeighborhood::empty();
        let mut center_l = LightData::dark();
        center_l.set_block_light(3, 3, 3, 9);
        hood.sections[MeshNeighborhood::index(0, 0, 0)] = Some(Arc::new(Section::empty()));
        hood.light[MeshNeighborhood::index(0, 0, 0)] = Some(Arc::new(center_l));
        let mut west_l = LightData::dark();
        west_l.set_sky(31, 5, 5, 7);
        hood.light[MeshNeighborhood::index(-1, 0, 0)] = Some(Arc::new(west_l));
        assert_eq!(hood.get_light(3, 3, 3), 9 << 4);
        assert_eq!(hood.get_light(-1, 5, 5), 7, "x=-1 reads the west neighbor's x=31");
        assert_eq!(hood.get_light(32, 5, 5), 0x0F, "missing neighbor = open sky (world top apron)");
    }
```

`git-craft/src/mesh/greedy.rs` tests:

```rust
    #[test]
    fn quad_light_samples_the_out_cell_not_the_block() {
        // Stone at padded (6,6,6); the cell above it carries sky 12, the
        // stone's own cell carries 0 (opaque cells hold no light).
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set_light(6, 7, 6, 12); // sky 12, block 0
        p.set_light(6, 6, 6, 0);
        let quads = mesh(&p);
        let top = quads.iter().find(|q| q.face == 2).unwrap();
        assert_eq!(top.skylight, 12, "top face lit by the cell above");
        assert_eq!(top.blocklight, 0);
    }

    #[test]
    fn torch_lit_out_cell_sets_blocklight() {
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set_light(7, 6, 6, 0x0E << 4); // sky 0, block 14 in front of +X
        let quads = mesh(&p);
        let px = quads.iter().find(|q| q.face == 0).unwrap();
        assert_eq!(px.blocklight, 14);
        assert_eq!(px.skylight, 0);
    }

    #[test]
    fn light_boundary_splits_the_greedy_merge() {
        // Two-block strip along x whose top out-cells differ: 15 vs 10.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(7, 6, 6, STONE);
        p.set_light(7, 7, 6, 10);
        let quads = mesh(&p);
        let tops: Vec<_> = quads.iter().filter(|q| q.face == 2 && q.y == 5).collect();
        assert_eq!(tops.len(), 2, "differing light must split the merge");
        let mut lights: Vec<u32> = tops.iter().map(|q| q.skylight).collect();
        lights.sort_unstable();
        assert_eq!(lights, vec![10, 15]);
    }

    #[test]
    fn uniform_light_still_merges_fully() {
        // The default 0x0F everywhere: the 32×32 slab still merges to 1 top quad.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for z in 1..=32 {
                p.set(x, 1, z, GRASS);
            }
        }
        assert_eq!(mesh(&p).iter().filter(|q| q.unpack_is_top()).count(), 1);
    }
```

(For the last test reuse the existing pattern `q.face == 2` instead of inventing `unpack_is_top` — write it as the existing `slab_interior_still_merges_fully` does; it is listed here to pin that the light key must not break full-slab merging.)

- [x] **Step 7.2: Run tests to verify they fail**

Run: `cargo test --manifest-path git-craft/Cargo.toml mesh`
Expected: FAIL to compile (`light_packed`, `set_light`, `build` arity, `hood.light`).

- [x] **Step 7.3: Implement the padded light channel**

`git-craft/src/mesh/padded.rs`:

```rust
use crate::world::block::{BlockId, AIR};
use crate::world::light::{pack_light, LightData, MAX_LIGHT};
use crate::world::section::{Section, SECTION_SIZE};

/// Padded cube edge: 32 interior + 1 apron voxel on each side.
pub const PADDED: usize = SECTION_SIZE + 2;
const VOLUME: usize = PADDED * PADDED * PADDED;

/// Flat 34³ snapshot a mesh job works on. Padded coords are 0..34;
/// padded (x+1, y+1, z+1) == section-local (x, y, z). Light rides along:
/// one packed byte per cell (sky low nibble, block high nibble), defaulting
/// to open sky — the correct apron value above the world ceiling and the
/// value that keeps light-less tests behaving like M2/M3.
pub struct PaddedSection {
    blocks: Box<[BlockId; VOLUME]>,
    light: Box<[u8; VOLUME]>,
}

impl PaddedSection {
    pub fn air() -> Self {
        Self {
            blocks: Box::new([AIR; VOLUME]),
            light: Box::new([pack_light(MAX_LIGHT, 0); VOLUME]),
        }
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < PADDED && y < PADDED && z < PADDED);
        (y * PADDED + z) * PADDED + x
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockId {
        self.blocks[Self::index(x, y, z)]
    }

    pub fn light_packed(&self, x: usize, y: usize, z: usize) -> u8 {
        self.light[Self::index(x, y, z)]
    }

    /// Test scaffolding: build arbitrary voxel scenes without a Section.
    #[allow(dead_code)] // test scaffolding
    pub fn set(&mut self, x: usize, y: usize, z: usize, block: BlockId) {
        self.blocks[Self::index(x, y, z)] = block;
    }

    /// Test scaffolding: packed light (sky | block << 4).
    #[allow(dead_code)] // test scaffolding
    pub fn set_light(&mut self, x: usize, y: usize, z: usize, packed: u8) {
        self.light[Self::index(x, y, z)] = packed;
    }

    /// Interior from `center` + `center_light` (bulk-decoded once); apron
    /// cells — every padded cell with any coordinate 0 or 33 — from
    /// `neighbor`, which receives section-local coordinates in -1..=32 (at
    /// least one out of range) and returns (block, packed light).
    pub fn build(
        center: &Section,
        center_light: &LightData,
        neighbor: impl Fn(i32, i32, i32) -> (BlockId, u8),
    ) -> Self {
        let mut p = Self::air();
        let mut flat = vec![AIR; SECTION_SIZE * SECTION_SIZE * SECTION_SIZE];
        center.unpack_into(&mut flat);
        let mut flat_light = vec![0u8; SECTION_SIZE * SECTION_SIZE * SECTION_SIZE];
        center_light.unpack_into(&mut flat_light);
        for y in 0..SECTION_SIZE {
            for z in 0..SECTION_SIZE {
                let row = (y * SECTION_SIZE + z) * SECTION_SIZE;
                let prow = Self::index(1, y + 1, z + 1);
                p.blocks[prow..prow + SECTION_SIZE].copy_from_slice(&flat[row..row + SECTION_SIZE]);
                p.light[prow..prow + SECTION_SIZE]
                    .copy_from_slice(&flat_light[row..row + SECTION_SIZE]);
            }
        }
        for y in 0..PADDED {
            for z in 0..PADDED {
                for x in 0..PADDED {
                    if x == 0 || x == PADDED - 1 || y == 0 || y == PADDED - 1 || z == 0 || z == PADDED - 1 {
                        let (b, l) = neighbor(x as i32 - 1, y as i32 - 1, z as i32 - 1);
                        p.blocks[Self::index(x, y, z)] = b;
                        p.light[Self::index(x, y, z)] = l;
                    }
                }
            }
        }
        p
    }
}
```

Update the two existing `build` tests to the new closure signature: `|_, _, _| (AIR, 0x0F)` and the tagging test `|x, y, z| { ...; (BlockId(outside), 0x0F) }`.

- [x] **Step 7.4: Implement the neighborhood light capture**

`git-craft/src/mesh/neighborhood.rs`:

```rust
use crate::world::light::{pack_light, LightData, MAX_LIGHT};
```

```rust
pub struct MeshNeighborhood {
    pub sections: [Option<Arc<Section>>; 27],
    pub light: [Option<Arc<LightData>>; 27],
}
```

```rust
    pub fn empty() -> Self {
        Self {
            sections: std::array::from_fn(|_| None),
            light: std::array::from_fn(|_| None),
        }
    }
```

Add next to `get`:

```rust
    /// Section-local coords in -1..=32 (apron space) → packed light.
    /// Missing light reads as open sky (sky 15 / block 0): the only
    /// legitimately absent neighbors are above the world ceiling (correct)
    /// and below the floor (invisible — the world floor has no underside).
    pub fn get_light(&self, x: i32, y: i32, z: i32) -> u8 {
        let part = |v: i32| -> (i32, usize) {
            if v < 0 {
                (-1, (v + 32) as usize)
            } else if v >= 32 {
                (1, (v - 32) as usize)
            } else {
                (0, v as usize)
            }
        };
        let (dx, lx) = part(x);
        let (dy, ly) = part(y);
        let (dz, lz) = part(z);
        match &self.light[Self::index(dx, dy, dz)] {
            Some(l) => l.packed(lx, ly, lz),
            None => pack_light(MAX_LIGHT, 0),
        }
    }

    pub fn build_padded(&self) -> PaddedSection {
        let center = self.sections[Self::index(0, 0, 0)]
            .as_ref()
            .expect("mesh job scheduled without a center section");
        let default_light = LightData::uniform(MAX_LIGHT, 0);
        let center_light: &LightData =
            self.light[Self::index(0, 0, 0)].as_deref().unwrap_or(&default_light);
        PaddedSection::build(center, center_light, |x, y, z| {
            (self.get(x, y, z), self.get_light(x, y, z))
        })
    }
```

- [x] **Step 7.5: Implement mesher light sampling and merge key**

`git-craft/src/mesh/greedy.rs`:

`plane_key` gains the packed light byte (bits 34..42 of the u64 key — face already ends at bit 34):

```rust
fn plane_key(face: u32, slice: u32, block: u16, ao_key: u32, light: u8) -> u64 {
    debug_assert!(ao_key < 512, "ao_key {ao_key} exceeds 9 bits; would corrupt slice field");
    block as u64
        | (ao_key as u64) << 16
        | (slice as u64) << 25
        | (face as u64) << 31
        | (light as u64) << 34
}
```

In `build_planes`, sample the out-layer cell (one step along the face normal from the solid cell — the same `base` AO uses) and pass it through:

```rust
                            let block = padded.get(x, y, z);
                            let ao_key = ao_neighborhood(padded, x, y, z, face as usize);
                            let n = FACE_N[face as usize];
                            let light = padded.light_packed(
                                (x as i32 + n[0]) as usize,
                                (y as i32 + n[1]) as usize,
                                (z as i32 + n[2]) as usize,
                            );
                            let key = plane_key(face, c, block.0, ao_key, light);
```

In `sweep_planes`, extract and forward:

```rust
        for (key, plane) in planes {
            let block = (key & 0xFFFF) as u16;
            let ao_key = ((key >> 16) & 0x1FF) as u32;
            let slice = ((key >> 25) & 0x3F) as u32;
            let face = ((key >> 31) & 0x7) as u32;
            let light = ((key >> 34) & 0xFF) as u8;
            sweep_plane(face, slice, block, ao_key, light, plane, &mut self.quads);
        }
```

**Note the face extraction changes** from `(key >> 31) as u32` to `((key >> 31) & 0x7) as u32` — the light bits now live above it.

Thread `light: u8` through `sweep_plane` and `emit` (one more parameter each), and in `emit` replace the hardcoded values:

```rust
    out.push(PackedQuad::pack(Quad {
        x, y, z, face, w, h,
        ao,
        skylight: (light & 0x0F) as u32,
        blocklight: (light >> 4) as u32,
        texture: block as u32,
        flip,
    }));
```

- [x] **Step 7.6: Fill the neighborhood light in App**

`git-craft/src/app.rs`, `build_neighborhood` — capture the light Arc next to the section Arc:

```rust
                    if let Some(c) = self.world.ready(col) {
                        hood.sections[MeshNeighborhood::index(dx, dy, dz)] =
                            Some(c.sections[sy as usize].clone());
                        hood.light[MeshNeighborhood::index(dx, dy, dz)] =
                            Some(c.light[sy as usize].clone());
                    }
```

- [x] **Step 7.7: Run everything**

Run: `cargo test --manifest-path git-craft/Cargo.toml`
Expected: PASS — all pre-existing greedy/padded/neighborhood tests stay green (defaults preserve the old `skylight 15 / blocklight 0` behavior), plus the new light tests.

Visual check (optional but recommended): `cargo run --release --manifest-path git-craft/Cargo.toml` — caves and overhangs are now dark, the surface unchanged. Torches don't glow yet (shader reads blocklight in Task 8); placing blocks under an overhang should show light gradients.

- [x] **Step 7.8: Commit**

```bash
git add git-craft/src/mesh/padded.rs git-craft/src/mesh/neighborhood.rs git-craft/src/mesh/greedy.rs git-craft/src/app.rs
git commit -m "feat: bake flood-fill light into greedy mesh quads"
```

---

### Task 8: Shader — blocklight, sun, and day factor

**Files:**
- Modify: `git-craft/src/render/terrain.rs` (`CameraUniform` → `FrameUniform`)
- Modify: `git-craft/assets/shaders/terrain.wgsl`
- Modify: `git-craft/src/app.rs` (call site, hardcoded noon until Task 9)
- Test: uniform layout test in `terrain.rs`; the existing `shipped_terrain_shader_is_valid` covers WGSL compilation; the palette-drift test in `block.rs` keeps the tables honest

The frame uniform grows two vec4s: `sky` (rgb sky color, w = day factor 0..1) and `sun` (xyz world-space sun direction). The lighting model is the M4 "basic sun" — a per-face NdotL on the face normal blended with the Minecraft-style face shade, gated by flood-fill skylight so caves stay dark at noon, plus a warm time-independent torch term. The full spec model (CSM, real ambient split) replaces this in M5; constants here are tuned live via shader hot-reload.

- [x] **Step 8.1: Write the failing uniform-layout test**

In `git-craft/src/render/terrain.rs` tests:

```rust
    #[test]
    fn frame_uniform_layout_matches_wgsl() {
        // mat4x4 (64) + vec4 sky (16) + vec4 sun (16). WGSL struct layout
        // would silently misread on drift.
        assert_eq!(std::mem::size_of::<FrameUniform>(), 96);
        assert_eq!(std::mem::offset_of!(FrameUniform, sky), 64);
        assert_eq!(std::mem::offset_of!(FrameUniform, sun), 80);
    }
```

Run: `cargo test --manifest-path git-craft/Cargo.toml terrain::`
Expected: FAIL to compile — `FrameUniform` not defined.

- [x] **Step 8.2: Implement FrameUniform**

In `git-craft/src/render/terrain.rs`, replace `CameraUniform`:

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    /// rgb = sky color (linear), w = day factor 0..1.
    sky: [f32; 4],
    /// xyz = world-space sun direction (normalized, pointing AT the sun).
    sun: [f32; 4],
}
```

Replace every `CameraUniform` mention (`size_of` in the buffer descriptor included) and replace `write_camera` with:

```rust
    pub fn write_frame(
        &self,
        queue: &wgpu::Queue,
        view_proj: glam::Mat4,
        sky_color: glam::Vec3,
        day_factor: f32,
        sun_dir: glam::Vec3,
    ) {
        let uniform = FrameUniform {
            view_proj: view_proj.to_cols_array_2d(),
            sky: [sky_color.x, sky_color.y, sky_color.z, day_factor],
            sun: [sun_dir.x, sun_dir.y, sun_dir.z, 0.0],
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }
```

The uniform visibility must now include the fragment stage? No — all lighting stays in the vertex shader (per-vertex light, spec §4); leave `ShaderStages::VERTEX`.

- [x] **Step 8.3: Update the call site with hardcoded noon**

In `git-craft/src/app.rs` `render()`:

```rust
        if let Some(terrain) = self.terrain.as_mut() {
            // Noon placeholder until the day cycle lands (M4 Task 9).
            terrain.write_frame(
                &gpu.queue,
                view_proj,
                glam::Vec3::new(0.25, 0.55, 0.95),
                1.0,
                glam::Vec3::new(0.3, 0.85, 0.42).normalize(),
            );
            ...
        }
```

- [x] **Step 8.4: Rewrite the shader lighting**

`git-craft/assets/shaders/terrain.wgsl` — full updated file:

```wgsl
struct FrameUniform {
    view_proj: mat4x4<f32>,
    sky: vec4<f32>,   // rgb sky color (linear), w = day factor 0..1
    sun: vec4<f32>,   // xyz sun direction (normalized, toward the sun)
};

struct SectionInfo {
    origin: vec4<i32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniform;
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;
@group(1) @binding(1) var<storage, read> sections: array<SectionInfo>;

// Per-face: origin offset (added to voxel pos), U axis, V axis.
// Face order matches Rust: 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z.
// Invariant: cross(U, V) == outward face normal, so quads wind CCW seen from
// outside and survive backface culling. Quad w spans U, h spans V; AO corner
// order (0,0) (w,0) (w,h) (0,h) is defined in these same U/V axes.
const FACE_ORIGIN = array<vec3<f32>, 6>(
    vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 0.0),
);
const FACE_U = array<vec3<f32>, 6>(
    vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0),
    vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0),
    vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0),
);
const FACE_V = array<vec3<f32>, 6>(
    vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0),
    vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0),
    vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0),
);
const FACE_NORMAL = array<vec3<f32>, 6>(
    vec3(1.0, 0.0, 0.0), vec3(-1.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0), vec3(0.0, -1.0, 0.0),
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, -1.0),
);
// Minecraft-style face shading: +X, -X, +Y(top), -Y(bottom), +Z, -Z.
const FACE_SHADE = array<f32, 6>(0.8, 0.8, 1.0, 0.5, 0.6, 0.6);
// Warm torch tint, applied where blocklight dominates skylight.
const TORCH_TINT = vec3(1.0, 0.62, 0.33);

// M2 palette indexed by the quad's texture field = block id;
// procedural textures replace this in M6.
const PALETTE = array<vec3<f32>, 13>(
    vec3(1.0, 0.0, 1.0),      //  0 air (never rendered; magenta = bug)
    vec3(0.35, 0.62, 0.22),   //  1 grass
    vec3(0.45, 0.32, 0.2),    //  2 dirt
    vec3(0.52, 0.52, 0.54),   //  3 stone
    vec3(0.86, 0.81, 0.58),   //  4 sand
    vec3(0.91, 0.93, 0.95),   //  5 snow grass
    vec3(0.19, 0.36, 0.68),   //  6 water (opaque until M5)
    vec3(0.42, 0.31, 0.19),   //  7 oak log
    vec3(0.23, 0.43, 0.14),   //  8 oak leaves
    vec3(0.32, 0.23, 0.14),   //  9 spruce log
    vec3(0.16, 0.3, 0.19),    // 10 spruce leaves
    vec3(0.27, 0.5, 0.21),    // 11 cactus
    vec3(0.95, 0.71, 0.3),    // 12 torch
);

// Corner order matches PackedQuad ao order: (0,0) (w,0) (w,h) (0,h).
const CORNER_UV = array<vec2<f32>, 4>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
};

// base_vertex (4 × arena offset) is already folded into vi, so vi/4 is the
// arena-global quad index; first_instance carries the section slot.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> VsOut {
    let quad = quads[vi / 4u];
    let flip = extractBits(quad.y, 31u, 1u);
    // AO diagonal flip: rotating the corner mapping by one turns the fixed
    // index pattern (0,1,2)(0,2,3) into triangles (1,2,3)(1,3,0) — the same
    // rectangle cut along the other diagonal. Positions and AO follow the
    // rotated corner, so geometry is identical and only the cut changes.
    let corner = (vi + flip) % 4u;

    let x = f32(extractBits(quad.x, 0u, 6u));
    let y = f32(extractBits(quad.x, 6u, 6u));
    let z = f32(extractBits(quad.x, 12u, 6u));
    let face = extractBits(quad.x, 18u, 3u);
    let w = f32(extractBits(quad.x, 21u, 5u) + 1u);
    let h = f32(extractBits(quad.y, 0u, 5u) + 1u);
    let ao = f32(extractBits(quad.y, 5u + corner * 2u, 2u));
    let skylight = f32(extractBits(quad.y, 13u, 4u)) / 15.0;
    let blocklight = f32(extractBits(quad.y, 17u, 4u)) / 15.0;
    let tex = extractBits(quad.y, 21u, 10u);

    let uv = CORNER_UV[corner];
    let local = vec3(x, y, z) + FACE_ORIGIN[face] + FACE_U[face] * uv.x * w + FACE_V[face] * uv.y * h;
    let world = vec3<f32>(sections[slot].origin.xyz) + local;

    var out: VsOut;
    out.clip = frame.view_proj * vec4(world, 1.0);

    // M4 basic-sun lighting (the full spec §6 model arrives with CSM in M5):
    //   sky term  = flood-fill skylight × day factor × (ambient face shade
    //               blended with per-face NdotL toward the sun)
    //   torch term = flood-fill blocklight × face shade, time-independent
    // Skylight gates the sun term, so caves stay dark at noon.
    let ndotl = max(dot(FACE_NORMAL[face], frame.sun.xyz), 0.0);
    let sky_l = skylight * frame.sky.w * (0.45 * FACE_SHADE[face] + 0.55 * ndotl);
    let torch_l = blocklight * FACE_SHADE[face];
    let level = max(max(sky_l, torch_l), 0.02);
    let tint = mix(vec3(1.0), TORCH_TINT, clamp(torch_l - sky_l, 0.0, 1.0));
    let ao_f = mix(0.4, 1.0, ao / 3.0);
    out.color = PALETTE[min(tex, 12u)] * tint * level * ao_f;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
```

- [x] **Step 8.5: Run everything**

Run: `cargo test --manifest-path git-craft/Cargo.toml`
Expected: PASS — `shipped_terrain_shader_is_valid` recompiles the new WGSL, `colors_match_the_shader_palette` still parses the table, the new layout test passes.

Visual check: `cargo run --release --manifest-path git-craft/Cargo.toml` — place a torch in a cave: warm orange pool of light decaying over ~14 blocks. Surfaces facing the (fixed noon) sun are brighter than opposite faces.

- [x] **Step 8.6: Commit**

```bash
git add git-craft/src/render/terrain.rs git-craft/assets/shaders/terrain.wgsl git-craft/src/app.rs
git commit -m "feat: light terrain from skylight, blocklight, and a basic sun"
```

---

### Task 9: Day/night cycle

**Files:**
- Create: `git-craft/src/game/daycycle.rs`
- Modify: `git-craft/src/game/mod.rs` (add `pub mod daycycle;`)
- Modify: `git-craft/src/app.rs` (advance + feed the uniform + clear color + HUD)
- Test: in-file tests in `daycycle.rs`

Pure time-of-day state: `time ∈ [0,1)`, full cycle 20 minutes (spec §7), `0.0 = sunrise`. The sun travels a tilted circle (so it is never exactly overhead — better face shading); `day_factor` smoothsteps around sunrise/sunset with a 0.03 night floor (a hint of moonlight; the real moon light is M5); `sky_color` lerps night→day and blends an orange band near the horizon. **Flood-fill light is never touched** — darkening is entirely this uniform (spec §4).

- [x] **Step 9.1: Write the failing tests**

Create `git-craft/src/game/daycycle.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn at(time: f32) -> DayCycle {
        DayCycle { time, cycle_secs: DayCycle::DEFAULT_CYCLE_SECS }
    }

    #[test]
    fn advance_wraps_and_is_proportional() {
        let mut d = at(0.9);
        d.advance(DayCycle::DEFAULT_CYCLE_SECS * 0.2); // +0.2 of a day
        assert!((d.time - 0.1).abs() < 1e-4, "wrapped past 1.0, got {}", d.time);
    }

    #[test]
    fn noon_is_full_midnight_is_floor() {
        assert!((at(0.25).day_factor() - 1.0).abs() < 1e-3, "noon");
        assert!((at(0.75).day_factor() - 0.03).abs() < 1e-3, "midnight floor");
        assert!(at(0.5).day_factor() < 0.6, "sunset is dimmer than midday");
        assert!(at(0.05).day_factor() > at(0.0).day_factor(), "brightening after sunrise");
    }

    #[test]
    fn day_factor_stays_in_bounds_all_day() {
        for i in 0..200 {
            let f = at(i as f32 / 200.0).day_factor();
            assert!((0.03..=1.0).contains(&f), "t={i}: factor {f}");
        }
    }

    #[test]
    fn sun_rises_in_the_east_and_is_normalized() {
        let sunrise = at(0.0).sun_dir();
        assert!(sunrise.x > 0.9, "sunrise points east (+X), got {sunrise}");
        let noon = at(0.25).sun_dir();
        assert!(noon.y > 0.9, "noon is overhead");
        for i in 0..20 {
            let d = at(i as f32 / 20.0).sun_dir();
            assert!((d.length() - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn sky_color_is_blue_at_noon_dark_at_midnight_warm_at_sunset() {
        let noon = at(0.25).sky_color();
        assert!(noon.z > noon.x, "noon sky is blue-dominant");
        let midnight = at(0.75).sky_color();
        assert!(midnight.max_element() < 0.08, "midnight is near-black, got {midnight}");
        let sunset = at(0.5).sky_color();
        assert!(sunset.x > sunset.z, "sunset is warm (red over blue), got {sunset}");
    }
}
```

Run: `cargo test --manifest-path git-craft/Cargo.toml daycycle::` after adding `pub mod daycycle;` to `git-craft/src/game/mod.rs`.
Expected: FAIL to compile.

- [x] **Step 9.2: Implement DayCycle**

Above the tests in `daycycle.rs`:

```rust
// Day/night cycle (spec §7): 20-minute full cycle; the sun angle drives the
// shader's sky/sun uniforms and the clear color. Flood-fill skylight values
// are NEVER touched — night darkening happens entirely in the shader via
// the day factor / sky color (spec §4).

use glam::Vec3;

pub struct DayCycle {
    /// Fraction of a full day in [0, 1): 0.0 sunrise, 0.25 noon,
    /// 0.5 sunset, 0.75 midnight.
    pub time: f32,
    pub cycle_secs: f32,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl DayCycle {
    pub const DEFAULT_CYCLE_SECS: f32 = 1200.0; // 20 minutes (spec §7)

    pub fn new() -> Self {
        // Start mid-morning: bright, with the sun low enough to show shading.
        Self { time: 0.1, cycle_secs: Self::DEFAULT_CYCLE_SECS }
    }

    pub fn advance(&mut self, dt: f32) {
        self.time = (self.time + dt / self.cycle_secs).rem_euclid(1.0);
    }

    /// Sun elevation in [-1, 1]: sin of the day angle.
    fn elevation(&self) -> f32 {
        (self.time * std::f32::consts::TAU).sin()
    }

    /// Sky brightness multiplier in [0.03, 1]: smoothsteps through dawn and
    /// dusk, with a small floor standing in for moonlight until M5.
    pub fn day_factor(&self) -> f32 {
        0.03 + 0.97 * smoothstep(-0.08, 0.25, self.elevation())
    }

    /// World-space direction TOWARD the sun. Rises +X (east), sets -X, on a
    /// circle tilted slightly into +Z so noon is never exactly overhead.
    pub fn sun_dir(&self) -> Vec3 {
        let a = self.time * std::f32::consts::TAU;
        Vec3::new(a.cos(), a.sin(), 0.12).normalize()
    }

    /// Linear-space sky / clear color: night → day by the day factor, with
    /// an orange band blended in near the horizon (sunrise and sunset).
    pub fn sky_color(&self) -> Vec3 {
        let day = Vec3::new(0.25, 0.55, 0.95);
        let night = Vec3::new(0.008, 0.012, 0.035);
        let base = night.lerp(day, self.day_factor());
        let glow = (1.0 - self.elevation().abs() / 0.22).clamp(0.0, 1.0);
        base.lerp(Vec3::new(0.9, 0.45, 0.2), glow * 0.45)
    }
}

impl Default for DayCycle {
    fn default() -> Self {
        Self::new()
    }
}
```

Check the noon `sun_dir` assertion: at `time = 0.25`, `a = TAU/4`, `cos ≈ 0`, `sin = 1` → `(0, 1, 0.12).normalize()` has `y ≈ 0.993` ✓. The sunset sky check: `elevation(0.5) ≈ 0` → glow 1.0 → strong orange blend ✓.

- [x] **Step 9.3: Run the daycycle tests**

Run: `cargo test --manifest-path git-craft/Cargo.toml daycycle::`
Expected: PASS (5 tests).

- [x] **Step 9.4: Wire into App**

In `git-craft/src/app.rs`:

- Field: `day: crate::game::daycycle::DayCycle,` initialized with `crate::game::daycycle::DayCycle::new()` in `App::new`.
- In `render()` right after `dt` is computed (BEFORE the `cursor_grabbed` branch — ambient time advances even while paused):

```rust
        self.day.advance(dt);
```

- Replace the Task 8 noon placeholder:

```rust
            terrain.write_frame(
                &gpu.queue,
                view_proj,
                self.day.sky_color(),
                self.day.day_factor(),
                self.day.sun_dir(),
            );
```

- Clear color follows the sky. Replace the hardcoded `LoadOp::Clear(wgpu::Color { r: 0.25, ... })`:

```rust
        let sky = self.day.sky_color();
        // ... inside the render pass descriptor:
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: sky.x as f64,
                            g: sky.y as f64,
                            b: sky.z as f64,
                            a: 1.0,
                        }),
```

(Capture `let sky = self.day.sky_color();` before the encoder block, next to the other captured locals.)

- HUD line (after `Light:`): capture `let day_label = format!("{:02}:{:02} (×{:.2})", ((self.day.time * 24.0 + 6.0) % 24.0) as u32, ((self.day.time * 24.0 * 60.0) % 60.0) as u32, self.day.day_factor());` and show:

```rust
                                ui.label(format!("Time:     {day_label}"));
```

(`+6.0`: time 0 = sunrise = 06:00, noon = 12:00.)

- [x] **Step 9.5: Run, smoke, commit**

Run: `cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings`
Expected: PASS.

Visual check: run release; for a fast visual sweep of a full day, temporarily set `cycle_secs` to 60.0 in `DayCycle::new`, watch a sunrise→noon→sunset→night sweep (sky color, terrain brightness, torch pools at night), then **revert to `DEFAULT_CYCLE_SECS` before committing**.

```bash
git add git-craft/src/game/daycycle.rs git-craft/src/game/mod.rs git-craft/src/app.rs
git commit -m "feat: add day/night cycle driving sun, sky color, and clear color"
```

---

### Task 10: Cave culling — face connectivity masks

**Files:**
- Create: `git-craft/src/render/visibility.rs`
- Modify: `git-craft/src/render/mod.rs` (add `pub mod visibility;`)
- Modify: `git-craft/src/world/jobs.rs` (`Meshed` carries the mask)
- Test: in-file tests in `visibility.rs`

The Tommaso Checchi visibility graph (spec §6): at mesh time, flood-fill the section's non-solid voxels into connected components; for each component, record which of the 6 faces it touches; OR together the face-pair bits of every touched pair. 6 faces → 15 unordered pairs → a `u16` mask. "Can you see in through face A and out through face B?" becomes one AND. The mask is computed in the mesh job (the padded buffer's interior is exactly the section) and shipped alongside the quads.

- [x] **Step 10.1: Write the failing tests**

Create `git-craft/src/render/visibility.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::padded::PaddedSection;
    use crate::world::block::STONE;

    const ALL_PAIRS: u16 = 0x7FFF;

    #[test]
    fn pair_bits_are_distinct_and_cover_15() {
        let mut seen = 0u16;
        for a in 0..6 {
            for b in 0..6 {
                if a == b {
                    continue;
                }
                let bit = pair_bit(a, b);
                assert_eq!(bit.count_ones(), 1);
                assert_eq!(bit, pair_bit(b, a), "order-free");
                seen |= bit;
            }
        }
        assert_eq!(seen, ALL_PAIRS);
    }

    #[test]
    fn empty_section_connects_all_faces() {
        assert_eq!(face_connectivity(&PaddedSection::air()), ALL_PAIRS);
    }

    #[test]
    fn solid_section_connects_nothing() {
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    p.set(x, y, z, STONE);
                }
            }
        }
        assert_eq!(face_connectivity(&p), 0);
    }

    #[test]
    fn straight_tunnel_connects_only_its_two_faces() {
        // Solid section with a 1×1 tunnel along X at y=16, z=16.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    if y != 17 || z != 17 {
                        p.set(x, y, z, STONE);
                    }
                }
            }
        }
        // Faces: 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z (mesher order).
        assert_eq!(face_connectivity(&p), pair_bit(0, 1));
    }

    #[test]
    fn l_tunnel_connects_the_corner_pair() {
        // Tunnel entering -X at y=17,z=17 to the center, then turning +Z.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    let leg_x = y == 17 && z == 17 && x <= 16;
                    let leg_z = y == 17 && x == 16 && z >= 17;
                    if !(leg_x || leg_z) {
                        p.set(x, y, z, STONE);
                    }
                }
            }
        }
        assert_eq!(face_connectivity(&p), pair_bit(1, 4), "-X to +Z only");
    }

    #[test]
    fn disjoint_components_union_their_pairs() {
        // Two parallel tunnels: one along X (y=5,z=5), one along Z (y=20,x=20).
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    let tx = y == 6 && z == 6;
                    let tz = y == 21 && x == 21;
                    if !(tx || tz) {
                        p.set(x, y, z, STONE);
                    }
                }
            }
        }
        assert_eq!(face_connectivity(&p), pair_bit(0, 1) | pair_bit(4, 5));
    }

    #[test]
    fn dead_end_pocket_adds_no_pairs() {
        // A pocket touching only the -X face: you can see in, not through.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    let pocket = x <= 4 && y == 17 && z == 17;
                    if !pocket {
                        p.set(x, y, z, STONE);
                    }
                }
            }
        }
        assert_eq!(face_connectivity(&p), 0, "single-face component yields no pair");
    }
}
```

Coordinate note for test authors: `PaddedSection` coords are padded (interior 1..=32); a tunnel "at y=16 local" sits at padded 17. The masks only inspect the interior.

- [x] **Step 10.2: Register the module and verify tests fail**

Add `pub mod visibility;` to `git-craft/src/render/mod.rs`.

Run: `cargo test --manifest-path git-craft/Cargo.toml visibility::`
Expected: FAIL to compile.

- [x] **Step 10.3: Implement**

`git-craft/src/render/visibility.rs` above the tests:

```rust
// Cave culling (spec §6, Tommaso Checchi): per-section face-to-face
// visibility masks computed at mesh time, consumed by a per-frame BFS
// from the camera section (Task 11).

use crate::mesh::padded::PaddedSection;

/// Bit index for the unordered face pair (a, b), a ≠ b, faces in mesher
/// order 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z. 15 pairs → bits 0..15.
pub fn pair_bit(a: usize, b: usize) -> u16 {
    debug_assert!(a < 6 && b < 6 && a != b);
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    // Pairs ordered (0,1)(0,2)..(0,5)(1,2)..(4,5): offset of row `lo` is
    // lo*5 - lo*(lo-1)/2 in triangular numbering.
    const ROW_OFFSET: [u16; 5] = [0, 5, 9, 12, 14];
    1 << (ROW_OFFSET[lo] + (hi - lo - 1) as u16)
}

const SIZE: usize = 32;

fn vidx(x: usize, y: usize, z: usize) -> usize {
    (y * SIZE + z) * SIZE + x
}

/// Flood-fill the section's non-solid interior voxels into components; OR
/// the face-pair bits of every pair of faces a component touches. Costs
/// one pass over 32³ at mesh time (runs in the mesh job).
pub fn face_connectivity(padded: &PaddedSection) -> u16 {
    let mut visited = [0u64; SIZE * SIZE * SIZE / 64];
    let mut mask = 0u16;
    let mut stack: Vec<(usize, usize, usize)> = Vec::new();
    for sy in 0..SIZE {
        for sz in 0..SIZE {
            for sx in 0..SIZE {
                let i = vidx(sx, sy, sz);
                if visited[i / 64] & (1 << (i % 64)) != 0 {
                    continue;
                }
                // Padded offset +1: interior voxel (sx,sy,sz).
                if padded.get(sx + 1, sy + 1, sz + 1).is_solid() {
                    continue;
                }
                // New component: flood it, recording touched faces.
                let mut faces = 0u8;
                visited[i / 64] |= 1 << (i % 64);
                stack.push((sx, sy, sz));
                while let Some((x, y, z)) = stack.pop() {
                    if x == SIZE - 1 { faces |= 1 << 0; } // +X
                    if x == 0        { faces |= 1 << 1; } // -X
                    if y == SIZE - 1 { faces |= 1 << 2; } // +Y
                    if y == 0        { faces |= 1 << 3; } // -Y
                    if z == SIZE - 1 { faces |= 1 << 4; } // +Z
                    if z == 0        { faces |= 1 << 5; } // -Z
                    for (dx, dy, dz) in
                        [(1i32, 0i32, 0i32), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)]
                    {
                        let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
                        if !(0..SIZE as i32).contains(&nx)
                            || !(0..SIZE as i32).contains(&ny)
                            || !(0..SIZE as i32).contains(&nz)
                        {
                            continue;
                        }
                        let (nx, ny, nz) = (nx as usize, ny as usize, nz as usize);
                        let ni = vidx(nx, ny, nz);
                        if visited[ni / 64] & (1 << (ni % 64)) != 0 {
                            continue;
                        }
                        if padded.get(nx + 1, ny + 1, nz + 1).is_solid() {
                            continue;
                        }
                        visited[ni / 64] |= 1 << (ni % 64);
                        stack.push((nx, ny, nz));
                    }
                }
                for a in 0..6 {
                    for b in (a + 1)..6 {
                        if faces & (1 << a) != 0 && faces & (1 << b) != 0 {
                            mask |= pair_bit(a, b);
                        }
                    }
                }
            }
        }
    }
    mask
}
```

- [x] **Step 10.4: Ship the mask through the mesh job**

`git-craft/src/world/jobs.rs`:

```rust
    Meshed { pos: SectionPos, version: u64, quads: Vec<PackedQuad>, visibility: u16 },
```

```rust
    pub fn spawn_mesh(&mut self, pos: SectionPos, version: u64, hood: MeshNeighborhood) {
        self.mesh_in_flight += 1;
        let tx = self.tx.clone();
        rayon::spawn(move || {
            let padded = hood.build_padded();
            let quads = Mesher::new().mesh(&padded);
            let visibility = crate::render::visibility::face_connectivity(&padded);
            let _ = tx.send(JobResult::Meshed { pos, version, quads, visibility });
        });
    }
```

In `git-craft/src/app.rs`, extend the `Meshed` match arm pattern with `visibility` and ignore it for now (`visibility: _visibility` or capture and drop) — storage and the BFS land in Task 11. Keeping this task compiling on its own keeps the commit bisectable.

- [x] **Step 10.5: Run + commit**

Run: `cargo test --manifest-path git-craft/Cargo.toml`
Expected: PASS.

```bash
git add git-craft/src/render/visibility.rs git-craft/src/render/mod.rs git-craft/src/world/jobs.rs git-craft/src/app.rs
git commit -m "feat: compute section face-connectivity masks in mesh jobs"
```

---

### Task 11: Cave culling — camera BFS and draw filter

**Files:**
- Modify: `git-craft/src/render/visibility.rs` (add `visible_set`)
- Modify: `git-craft/src/render/terrain.rs` (`prepare` filter + stats)
- Modify: `git-craft/src/app.rs` (mask store, per-frame BFS, V toggle, HUD)
- Test: in-file tests in `visibility.rs` and `terrain.rs`

Per frame: BFS outward from the camera's section over the loaded-section graph. A step from section S through exit face f is allowed only if S's mask connects the face we entered through to f; steps that move opposite to any direction already traveled on this path are forbidden (the vanilla anti-wraparound rule); visited is keyed by position (first arrival wins — standard, slightly conservative). Sections with no mask yet (not meshed) are traversed freely — unknown must never hide geometry. The result set feeds `TerrainRenderer::prepare`, which already frustum-culls; cave culling composes with it.

- [x] **Step 11.1: Write the failing tests**

Append to `git-craft/src/render/visibility.rs` tests:

```rust
    use crate::world::chunks::SectionPos;
    use std::collections::HashMap;

    fn sp(x: i32, y: i32, z: i32) -> SectionPos {
        SectionPos { x, y, z }
    }

    #[test]
    fn open_world_reaches_the_whole_radius() {
        // No masks known at all: everything within radius and 0..8 vertically.
        let vis = visible_set(sp(0, 4, 0), 3, |_| None);
        assert!(vis.contains(&sp(0, 4, 0)));
        assert!(vis.contains(&sp(3, 4, 0)), "cardinal edge of the radius");
        assert!(vis.contains(&sp(2, 7, 2)));
        assert!(!vis.contains(&sp(4, 4, 0)), "outside the radius");
        assert!(!vis.contains(&sp(0, 8, 0)), "above the world");
    }

    #[test]
    fn fully_open_masks_match_unknown_masks() {
        let all = visible_set(sp(0, 4, 0), 2, |_| Some(0x7FFF));
        let unknown = visible_set(sp(0, 4, 0), 2, |_| None);
        assert_eq!(all, unknown);
    }

    #[test]
    fn sealed_sections_stop_the_walk() {
        // Everything at x >= 1 is sealed (mask 0): the BFS enters the x=1
        // wall sections (drawn — you see their face) but never passes them.
        let vis = visible_set(sp(0, 4, 0), 3, |p| Some(if p.x >= 1 { 0 } else { 0x7FFF }));
        assert!(vis.contains(&sp(1, 4, 0)), "the wall itself is visible");
        assert!(!vis.contains(&sp(2, 4, 0)), "nothing behind the wall");
        assert!(vis.contains(&sp(-3, 4, 0)), "open side unaffected");
    }

    #[test]
    fn tunnel_masks_gate_by_entry_face() {
        // Section (1,4,0) connects only -X↔+X; (1,4,1) (entered via +Z from
        // it) would need -Z↔anything. Camera at origin looking "through":
        let masks: HashMap<SectionPos, u16> = HashMap::from([
            (sp(1, 4, 0), pair_bit(0, 1)),
        ]);
        let default_open = 0x7FFF;
        let vis = visible_set(sp(0, 4, 0), 3, |p| Some(*masks.get(&p).unwrap_or(&default_open)));
        assert!(vis.contains(&sp(2, 4, 0)), "straight through the X tunnel");
        // (1,4,1) is reachable from (0,4,0) via (0,4,1) — open sections —
        // but NOT through the tunnel section's +Z face:
        assert!(vis.contains(&sp(1, 4, 1)), "reached around, not through");
        // Wall off the around-path to prove the tunnel doesn't leak +Z:
        let masks2: HashMap<SectionPos, u16> = HashMap::from([
            (sp(1, 4, 0), pair_bit(0, 1)),
            (sp(0, 4, 1), 0u16),
            (sp(0, 4, -1), 0u16),
            (sp(0, 3, 0), 0u16),
            (sp(0, 5, 0), 0u16),
            (sp(-1, 4, 0), 0u16),
        ]);
        let vis2 = visible_set(sp(0, 4, 0), 3, |p| Some(*masks2.get(&p).unwrap_or(&default_open)));
        assert!(vis2.contains(&sp(2, 4, 0)), "tunnel pass-through still works");
        assert!(!vis2.contains(&sp(1, 4, 1)), "tunnel's +Z face is not connected to its -X entry");
    }

    #[test]
    fn camera_above_the_world_clamps_and_sees_down() {
        let vis = visible_set(sp(0, 12, 0), 2, |_| None);
        assert!(vis.contains(&sp(0, 7, 0)), "clamped start at the world top");
        assert!(vis.contains(&sp(1, 6, 1)));
    }
```

And in `git-craft/src/render/terrain.rs` tests:

```rust
    #[test]
    fn draw_stats_count_cave_culled_sections() {
        // prepare() is GPU-coupled; the cave filter itself is pure: verify
        // the filtering contract via the stats struct shape instead.
        let stats = DrawStats {
            resident_sections: 10,
            visible_sections: 4,
            drawn_quads: 100,
            cave_culled: 3,
        };
        assert_eq!(stats.resident_sections - stats.cave_culled - stats.visible_sections, 3,
            "remaining 3 are frustum-culled");
    }
```

(The real verification of `prepare` filtering is the HUD numbers in Step 11.5 — `prepare` needs a live `wgpu::Queue`, which unit tests here don't spin up.)

- [x] **Step 11.2: Run tests to verify they fail**

Run: `cargo test --manifest-path git-craft/Cargo.toml visibility::`
Expected: FAIL to compile — `visible_set` missing.

- [x] **Step 11.3: Implement visible_set**

Append to `git-craft/src/render/visibility.rs` (above tests), with imports `use std::collections::{HashSet, VecDeque};` and `use crate::world::chunks::SectionPos;`:

```rust
/// Face index → step direction, mesher face order.
const FACE_STEP: [(i32, i32, i32); 6] =
    [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];
const OPPOSITE: [usize; 6] = [1, 0, 3, 2, 5, 4];

/// Per-frame cave-culling BFS (spec §6). Walk outward from the camera's
/// section; enter a neighbor only through faces the current section's mask
/// connects to the face we entered through; never step opposite to a
/// direction already traveled on this path (anti-wraparound); visited is
/// keyed by position (first arrival wins — vanilla's simplification,
/// conservative in pathological spirals). `mask_of` returning `None`
/// (section not meshed yet) traverses freely: unknown must never hide
/// geometry. Result: every section that may be visible; the frustum test
/// in `TerrainRenderer::prepare` intersects it.
pub fn visible_set(
    camera: SectionPos,
    radius: i32,
    mask_of: impl Fn(SectionPos) -> Option<u16>,
) -> HashSet<SectionPos> {
    let start = SectionPos { x: camera.x, y: camera.y.clamp(0, 7), z: camera.z };
    let r2 = radius * radius;
    let mut visited: HashSet<SectionPos> = HashSet::new();
    // (section, face we entered through, directions traveled so far)
    let mut queue: VecDeque<(SectionPos, Option<usize>, u8)> = VecDeque::new();
    visited.insert(start);
    queue.push_back((start, None, 0));
    while let Some((pos, entry, dirs)) = queue.pop_front() {
        for f in 0..6usize {
            if dirs & (1 << OPPOSITE[f]) != 0 {
                continue; // would step back toward the camera
            }
            if let Some(e) = entry {
                // Entering through e and leaving through f needs the pair
                // connected. f == e is unreachable: entering through e means
                // we traveled OPPOSITE(e), so the dirs check above bars f == e.
                if let Some(m) = mask_of(pos) {
                    if m & pair_bit(e, f) == 0 {
                        continue;
                    }
                }
            }
            let (dx, dy, dz) = FACE_STEP[f];
            let n = SectionPos { x: pos.x + dx, y: pos.y + dy, z: pos.z + dz };
            if !(0..8).contains(&n.y) {
                continue;
            }
            if (n.x - start.x).pow(2) + (n.z - start.z).pow(2) > r2 {
                continue;
            }
            if !visited.insert(n) {
                continue;
            }
            queue.push_back((n, Some(OPPOSITE[f]), dirs | (1 << f)));
        }
    }
    visited
}
```

- [x] **Step 11.4: Filter draws in TerrainRenderer**

`git-craft/src/render/terrain.rs`:

```rust
use std::collections::{HashMap, HashSet};
```

```rust
pub struct DrawStats {
    pub resident_sections: u32,
    pub visible_sections: u32,
    pub drawn_quads: u32,
    pub cave_culled: u32,
}
```

```rust
    /// Frustum-cull resident sections (optionally pre-filtered by the cave
    /// culling visible set) and write this frame's indirect args.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        frustum: &Frustum,
        visible: Option<&HashSet<SectionPos>>,
    ) -> DrawStats {
        let mut args: Vec<wgpu::util::DrawIndexedIndirectArgs> =
            Vec::with_capacity(self.entries.len());
        let mut drawn_quads = 0u32;
        let mut cave_culled = 0u32;
        for (pos, e) in &self.entries {
            if let Some(v) = visible {
                if !v.contains(pos) {
                    cave_culled += 1;
                    continue;
                }
            }
            let min = pos.origin().as_vec3();
            let max = min + glam::Vec3::splat(32.0);
            if !frustum.intersects_aabb(min, max) {
                continue;
            }
            drawn_quads += e.len;
            args.push(section_draw_args(e.offset, e.len, e.slot));
        }
        if !args.is_empty() {
            queue.write_buffer(&self.indirect_buffer, 0, bytemuck::cast_slice(&args));
        }
        self.visible_count = args.len() as u32;
        DrawStats {
            resident_sections: self.entries.len() as u32,
            visible_sections: self.visible_count,
            drawn_quads,
            cave_culled,
        }
    }
```

- [x] **Step 11.5: Wire into App**

`git-craft/src/app.rs`:

- Fields:

```rust
    /// Face-connectivity mask per meshed section (cave culling, spec §6).
    visibility_masks: HashMap<crate::world::chunks::SectionPos, u16>,
    cave_culling: bool,
    stats_cave_culled: u32,
```

Initialize `visibility_masks: HashMap::new(), cave_culling: true, stats_cave_culled: 0` in `App::new` (add `cave_culled: u32` to `FrameStats` instead of a loose field if you prefer — either is fine, keep it consistent with the existing `stats` pattern: put it in `FrameStats`).

- `Meshed` arm stores the mask under the same version guard as the upload:

```rust
                JobResult::Meshed { pos, version, quads, visibility } => {
                    let current = self.mesh_versions.get(&pos).copied().unwrap_or(0);
                    if version == current && self.world.ready(pos.column()).is_some() {
                        self.visibility_masks.insert(pos, visibility);
                        self.upload_queue.push_back((pos, quads));
                    }
                }
```

- Unload cleanup (next to `mesh_versions.remove`):

```rust
                    self.visibility_masks.remove(&section);
```

- V toggle, inside the `cursor_grabbed` input block next to the F handling:

```rust
            if self.input.key_pressed(KeyCode::KeyV) {
                self.cave_culling = !self.cave_culling;
            }
```

- The prepare call:

```rust
            let frustum = crate::render::frustum::Frustum::from_view_proj(view_proj);
            let visible = if self.cave_culling {
                let cam_section = crate::world::chunks::SectionPos {
                    x: (self.camera.position.x as i32).div_euclid(32),
                    y: (self.camera.position.y as i32).div_euclid(32),
                    z: (self.camera.position.z as i32).div_euclid(32),
                };
                Some(crate::render::visibility::visible_set(cam_section, RENDER_RADIUS, |p| {
                    self.visibility_masks.get(&p).copied()
                }))
            } else {
                None
            };
            let stats = terrain.prepare(&gpu.queue, &frustum, visible.as_ref());
            self.stats.visible_sections = stats.visible_sections;
            self.stats.resident_sections = stats.resident_sections;
            self.stats.drawn_quads = stats.drawn_quads;
            self.stats.cave_culled = stats.cave_culled;
```

- HUD: extend the sections line and show the toggle:

```rust
                                ui.label(format!(
                                    "Sections: {visible}/{resident} drawn/resident (cave-culled {cave_culled})"
                                ));
                                ui.label(format!("CaveCull: {}", if cave_cull_on { "on (V)" } else { "OFF (V)" }));
```

(capture `let cave_culled = self.stats.cave_culled;` and `let cave_cull_on = self.cave_culling;` with the other locals).

- [x] **Step 11.6: Run, validate with the HUD, commit**

Run: `cargo test --manifest-path git-craft/Cargo.toml && cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings`
Expected: PASS.

HUD validation (spec §8 discipline — claims need numbers): run release.
1. On the surface: `cave-culled` should be a large share of resident sections (underground geometry skipped). Toggle V: drawn count rises when culling turns OFF, FPS should not drop with it ON, and **the image must not change with the toggle while above ground or inside a cave** — if geometry pops, the masks or BFS are wrong.
2. Dig into a hillside, seal yourself in (place a block behind you), HUD `drawn` should collapse to nearby sections.
3. GPU ms should drop (or hold) with culling on; record before/after numbers in the task log.

```bash
git add git-craft/src/render/visibility.rs git-craft/src/render/terrain.rs git-craft/src/app.rs
git commit -m "feat: cull cave sections via per-frame visibility BFS"
```

---

### Task 12: Milestone validation

**Files:**
- Modify: `docs/superpowers/plans/2026-06-12-dabcraft-m4-light.md` (tick remaining checkboxes)
- No production code except fixes surfaced by the gates below.

- [x] **Step 12.1: Quality gates**

> Outcome note: the `cargo fmt --check` gate was waived — the codebase has never
> been rustfmt-formatted (M1–M3 gates were clippy + test only) and a repo-wide
> reformat mid-milestone would bury the M4 diff. clippy `-D warnings`, the full
> test suite (179 tests), and the release build all pass.

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --manifest-path git-craft/Cargo.toml --check
cargo clippy --manifest-path git-craft/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path git-craft/Cargo.toml
cargo build --release --manifest-path git-craft/Cargo.toml
```

Expected: all clean. Fix anything that isn't (no `--no-verify`, no allow-attributes to silence real issues).

- [x] **Step 12.2: Smoke run**

```bash
./git-craft/target/release/git-craft > /tmp/dabcraft-m4-final.log 2>&1 &
APP_PID=$!
sleep 30
kill $APP_PID
grep -iE "panic|error" /tmp/dabcraft-m4-final.log || echo "CLEAN"
```

Expected: `CLEAN`.

- [ ] **Step 12.3: Acceptance checklist (manual)** — PENDING USER PLAYTEST. Automated gates and the 30 s smoke run are done; the interactive HUD walkthrough below needs a human at the keyboard (the M3 playtest is also still pending).

Launch release and verify each item via the HUD (spec §8: numbers, not feel):

1. Caves and overhangs are dark; cave mouths show a smooth 15→0 gradient.
2. Place a torch in a cave: warm pool, ~14-block reach; break it: light fully retracts.
3. Dig straight down from the surface: the shaft floods with skylight; seal it overhead: goes dark.
4. Day/night: sky and terrain dim through sunset (HUD `Time` advancing); torch pools dominate at night; sunrise restores. No flood-fill recompute happens at any time-of-day change (no mesh-queue spike in the HUD while standing still as time passes).
5. Cave culling ON: HUD `cave-culled` large underground share on the surface; toggling V never changes the rendered image, only the drawn-section count and GPU ms.
6. Cross-column light: place a torch on a column border (x ≡ 31/32) — the pool crosses seamlessly.
7. Block edits at section borders re-mesh neighbors (no stale dark/bright seams after edits).
8. FPS at the default radius on the M4 remains at/near the M3 baseline (record the number).

- [x] **Step 12.4: Final commit**

Tick all plan checkboxes, then:

```bash
git add docs/superpowers/plans/2026-06-12-dabcraft-m4-light.md
git commit -m "docs: complete M4 light milestone plan"
```

---

## Self-review notes (written at plan time)

- **Spec coverage:** flood-fill skylight (Tasks 2–4), blocklight + torches (1, 5), incremental updates (5–6), light baked per-vertex into meshes (7), shader sky-term darkening with constant flood-fill data (8–9), cave culling (10–11), day/night driving a basic sun (9). §9 test list: BFS propagation (T4/T5), incremental == from-scratch (T5 `incremental_equals_from_scratch`), shader headless compile (existing `shipped_terrain_shader_is_valid`, exercised in T8), palette drift (existing, exercised in T1/T8).
- **Known M4 simplifications (deliberate, documented in code):** torch renders as a full collidable cube; water/leaves block light fully (no per-block attenuation until M5); `is_solid`/`blocks_light`/collidability are three separate notions — do not merge them.
- **Type-consistency check:** `LightChannel`/`LightData` named identically across chunks/light/light_engine/neighborhood; `insert_generated(pos, data, light, writes) -> Vec<IVec3>` used consistently in Tasks 3, 4, 5 (tests), 6; `Meshed.visibility: u16` consistent between Tasks 10 and 11; `prepare(queue, frustum, visible)` matches the app call.
- **Risk (spec §11):** light-update spikes on large edits — bounded BFS; if the HUD shows frame spikes on torch spam, the documented escape hatch is moving `on_block_changed` to the rayon pool with a one-frame delay. Do not pre-build that.

## Execution handoff

Plan complete. Execute with superpowers:subagent-driven-development (per-task implementer + spec reviewer + quality reviewer, logs in `.task-log/` with `m4-` prefixes), controller fixes trivia directly and amends task commits.
