---
title: dabcraft M2 — Infinite World
date: 2026-06-11
domain: world-layer
type: enhancement
priority: high
breaking: false
db-migration: false
rls-affecting: false
slice: 2
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files:
  - dabcraft/Cargo.toml
  - dabcraft/src/world/*.rs
  - dabcraft/src/mesh/*.rs
  - dabcraft/src/render/*.rs
  - dabcraft/src/game/camera.rs
  - dabcraft/src/app.rs
  - dabcraft/assets/shaders/terrain.wgsl
trigger-tasks-touched: []
shared-modules-touched: []
---

# dabcraft M2 — Infinite World Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Free flight through an infinite procedurally generated world (biomes, caves, trees) at 384-block render distance: chunk streaming around the camera, binary greedy meshing with real per-vertex AO on a rayon worker pool, arena-allocated GPU buffers with one `draw_indexed_indirect` per visible section, and CPU frustum + backface culling.

**Architecture:** The world is a `HashMap<ColumnPos, Column>` of 32-block columns (8 vertical 32³ sections, palette-compressed). Worldgen and meshing run as fire-and-forget `rayon::spawn` jobs that send results back over a crossbeam channel; the main thread drains results under per-frame budgets. Mesh quads live in one large arena storage buffer (slab allocator + free list); each resident section owns an arena range and a stable "slot" carrying its world origin in a second storage buffer, addressed in the vertex shader via `instance_index` (`first_instance` = slot, requires `Features::INDIRECT_FIRST_INSTANCE`). Per frame the CPU frustum-culls section AABBs and writes one `DrawIndexedIndirectArgs` per visible section.

**Tech Stack:** Rust (edition 2024), wgpu 29, fastnoise-lite 1.1, rayon 1, crossbeam-channel 0.5, glam 0.33, bytemuck 1.25.

**Spec:** `docs/superpowers/specs/2026-06-11-dabcraft-design.md` §4 (world system), §5 (meshing), §6 (culling, arena/indirect), §10 (M2). Lighting flood-fill is M4: M2 bakes `skylight = 15`, `blocklight = 0` into every quad. Water is rendered **opaque** in M2 (transparent pass is M5); leaves are opaque cubes (alpha-test pipeline arrives with textures in M6).

**No git remote exists** — skip all push/PR/issue steps. Commit locally on branch `feat/m2-world`.

**Environment:** every shell needs `export PATH="$HOME/.cargo/bin:$PATH"` before cargo commands. All commands run with `--manifest-path dabcraft/Cargo.toml` from the repo root (or `cd dabcraft` first).

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `dabcraft/Cargo.toml` | modify | add rayon, crossbeam-channel, fastnoise-lite |
| `dabcraft/src/world/block.rs` | modify | block registry grows to 12 ids |
| `dabcraft/src/world/section.rs` | rewrite | palette-compressed 32³ storage (same get/set API) |
| `dabcraft/src/world/chunks.rs` | create | ColumnPos/SectionPos, Column, ChunkMap, pending structure writes, radius helpers |
| `dabcraft/src/world/gen.rs` | create | WorldGen: heightmap, biomes, caves, column fill |
| `dabcraft/src/world/decor.rs` | create | trees/cacti decoration, StructureWrite |
| `dabcraft/src/world/jobs.rs` | create | rayon spawn + crossbeam result channel |
| `dabcraft/src/mesh/quad.rs` | modify | add AO diagonal-flip bit (data1 bit 31) |
| `dabcraft/src/mesh/padded.rs` | create | 34³ padded buffer with neighbor apron |
| `dabcraft/src/mesh/neighborhood.rs` | create | 3×3×3 `Arc<Section>` capture for mesh jobs |
| `dabcraft/src/mesh/greedy.rs` | create | binary greedy mesher with per-corner AO |
| `dabcraft/src/mesh/naive.rs` | **delete** (Task 14) | superseded by greedy |
| `dabcraft/src/render/frustum.rs` | create | plane extraction + AABB test |
| `dabcraft/src/render/arena.rs` | create | offset/len slab allocator with free-list |
| `dabcraft/src/render/terrain.rs` | rewrite | arena buffers, section slots, indirect draws |
| `dabcraft/src/render/gpu.rs` | modify | require `INDIRECT_FIRST_INSTANCE` |
| `dabcraft/src/game/camera.rs` | modify | far plane 1200, sprint multiplier |
| `dabcraft/src/app.rs` | rewrite | world streaming orchestration, HUD counters |
| `dabcraft/assets/shaders/terrain.wgsl` | rewrite | section origins via instance_index, flip bit, 12-color palette |

## Shared Conventions (read before any task)

- **Face order (unchanged from M1):** 0=+X 1=−X 2=+Y 3=−Y 4=+Z 5=−Z.
- **Face axes (unchanged, shipped `terrain.wgsl` is authoritative; the M1 plan document has an erratum):**

  | face | U axis | V axis |
  |---|---|---|
  | 0 (+X) | Y | Z |
  | 1 (−X) | Z | Y |
  | 2 (+Y) | Z | X |
  | 3 (−Y) | X | Z |
  | 4 (+Z) | X | Y |
  | 5 (−Z) | Y | X |

  Invariant: `cross(U, V) == outward normal`. Quad `w` spans U, `h` spans V. AO corner order in U/V space: `(0,0) (w,0) (w,h) (0,h)`.
- **PackedQuad bit layout** (M2 adds exactly one field, `flip`, in the previously unused data1 bit 31):
  - data0: `x@0:6 | y@6:6 | z@12:6 | face@18:3 | (w-1)@21:5`
  - data1: `(h-1)@0:5 | ao@5:8 | skylight@13:4 | blocklight@17:4 | texture@21:10 | flip@31:1`
- **Coordinates:** sections are 32³. `SectionPos{x,y,z}` are section coords (world block = pos*32); `y ∈ 0..8` (world height 256). `ColumnPos{x,z}`. Block→column uses `div_euclid(32)` (floor division — never `/` on negatives).
- **Quad positions** emitted by the mesher are section-local interior coords `0..32`; the shader adds the section's world origin.
- M2 constants (defined in Task 14, listed here for context): `RENDER_RADIUS = 12` columns (384 blocks), `LOAD_RADIUS = 13` (meshing needs 3×3 generated columns), `SEED = 1337`.
- After every task: `cargo test` green, then `cargo clippy -- -D warnings` clean, then commit. Never `--no-verify`, no Claude co-author trailers.

---

### Task 1: Dependencies + block registry expansion

**Files:**
- Modify: `dabcraft/Cargo.toml`
- Modify: `dabcraft/src/world/block.rs`

- [ ] **Step 1: Add dependencies**

In `dabcraft/Cargo.toml` `[dependencies]`, add:

```toml
rayon = "1.12"
crossbeam-channel = "0.5"
fastnoise-lite = "1.1"
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build --manifest-path dabcraft/Cargo.toml`
Expected: compiles (new deps fetched).

- [ ] **Step 3: Write failing tests for the new registry**

Append to the `tests` module of `dabcraft/src/world/block.rs` (create the module if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable() {
        // Persisted worlds (M6) depend on these exact values; never renumber.
        let expected: [(BlockId, u16); 12] = [
            (AIR, 0), (GRASS, 1), (DIRT, 2), (STONE, 3),
            (SAND, 4), (SNOW_GRASS, 5), (WATER, 6),
            (OAK_LOG, 7), (OAK_LEAVES, 8), (SPRUCE_LOG, 9),
            (SPRUCE_LEAVES, 10), (CACTUS, 11),
        ];
        for (block, id) in expected {
            assert_eq!(block.0, id);
        }
    }

    #[test]
    fn only_air_is_not_solid() {
        assert!(!AIR.is_solid());
        for id in 1..=11u16 {
            assert!(BlockId(id).is_solid());
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml world::block`
Expected: FAIL — `SAND` etc. not found.

- [ ] **Step 5: Implement the registry**

`dabcraft/src/world/block.rs` — replace the constants section (keep struct + `is_solid`):

```rust
pub const AIR: BlockId = BlockId(0);
pub const GRASS: BlockId = BlockId(1);
pub const DIRT: BlockId = BlockId(2);
pub const STONE: BlockId = BlockId(3);
pub const SAND: BlockId = BlockId(4);
pub const SNOW_GRASS: BlockId = BlockId(5);
// M2 renders water as an opaque solid; it moves to the transparent pass in M5.
pub const WATER: BlockId = BlockId(6);
pub const OAK_LOG: BlockId = BlockId(7);
pub const OAK_LEAVES: BlockId = BlockId(8);
pub const SPRUCE_LOG: BlockId = BlockId(9);
pub const SPRUCE_LEAVES: BlockId = BlockId(10);
pub const CACTUS: BlockId = BlockId(11);
```

Also add `Hash` to the derive list on `BlockId` (greedy meshing uses block ids in hash keys):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u16);
```

- [ ] **Step 6: Run tests, clippy**

Run: `cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`
Expected: all tests PASS (M1 suite still green), clippy clean.

- [ ] **Step 7: Commit**

```bash
git add dabcraft/Cargo.toml dabcraft/src/world/block.rs Cargo.lock dabcraft/Cargo.lock 2>/dev/null || git add dabcraft/Cargo.toml dabcraft/src/world/block.rs dabcraft/Cargo.lock
git commit -m "feat: add M2 dependencies and expand block registry to 12 ids"
```

(Whichever lockfile path exists; check `git status` first.)

---

### Task 2: Palette-compressed Section storage

Rewrite `Section`'s internals as palette + packed indices while keeping the `empty() / get / set / get_or_air` API, so existing callers (naive mesher, app) keep compiling. Adds `Clone`, semantic `PartialEq`, `uniform_block()`, `unpack_into()`, `compact()`.

**Files:**
- Rewrite: `dabcraft/src/world/section.rs`

**Design:** `palette: Vec<BlockId>` (`palette[0]` always exists and is the uniform fill), `bits: u32` (bits per voxel index; `0` = uniform, no voxel data), `data: Vec<u64>` (packed indices, entries may span word boundaries; `32768 * bits` is always a multiple of 64). `set` of a new block appends to the palette and repacks when `bits` no longer suffices. `compact()` rebuilds the palette from live content (shrink). An all-air section is `palette=[AIR], bits=0, data=[]` — zero voxel bytes, per spec §4.

- [ ] **Step 1: Write the failing tests**

Replace the `tests` module in `dabcraft/src/world/section.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{BlockId, AIR, DIRT, GRASS, STONE};

    #[test]
    fn set_then_get_roundtrips() {
        let mut s = Section::empty();
        s.set(31, 0, 17, STONE);
        assert_eq!(s.get(31, 0, 17), STONE);
        assert_eq!(s.get(0, 0, 0), AIR);
    }

    #[test]
    fn out_of_bounds_is_air() {
        let mut s = Section::empty();
        s.set(0, 0, 0, STONE);
        assert_eq!(s.get_or_air(-1, 0, 0), AIR);
        assert_eq!(s.get_or_air(0, 32, 0), AIR);
        assert_eq!(s.get_or_air(0, 0, 0), STONE);
    }

    #[test]
    fn empty_section_is_uniform_air_with_no_voxel_data() {
        let s = Section::empty();
        assert_eq!(s.uniform_block(), Some(AIR));
        assert_eq!(s.voxel_data_bytes(), 0);
    }

    #[test]
    fn palette_grows_through_repacks() {
        // 1→2 entries forces bits 0→1; 3 entries forces 1→2; 5 forces 2→3.
        let mut s = Section::empty();
        for (i, id) in (1..=8u16).enumerate() {
            s.set(i, 0, 0, BlockId(id));
        }
        for (i, id) in (1..=8u16).enumerate() {
            assert_eq!(s.get(i, 0, 0), BlockId(id), "voxel {i} after growth");
        }
        assert_eq!(s.get(20, 20, 20), AIR, "untouched voxels survive repacks");
        assert!(s.uniform_block().is_none());
    }

    #[test]
    fn every_voxel_roundtrips_with_word_spanning_indices() {
        // 3-bit indices: voxel index 21 spans the bit 63/64 word boundary.
        let mut s = Section::empty();
        let blocks = [AIR, GRASS, DIRT, STONE, BlockId(4)];
        for x in 0..32 {
            for y in 0..32 {
                for z in 0..32 {
                    s.set(x, y, z, blocks[(x * 7 + y * 3 + z) % 5]);
                }
            }
        }
        for x in 0..32 {
            for y in 0..32 {
                for z in 0..32 {
                    assert_eq!(s.get(x, y, z), blocks[(x * 7 + y * 3 + z) % 5]);
                }
            }
        }
    }

    #[test]
    fn compact_shrinks_palette_after_overwrites() {
        let mut s = Section::empty();
        for x in 0..32 {
            s.set(x, 0, 0, BlockId(x as u16 % 8));
        }
        for x in 0..32 {
            s.set(x, 0, 0, STONE); // orphan most palette entries
        }
        let before = s.palette_len();
        s.compact();
        assert!(s.palette_len() < before);
        assert_eq!(s.palette_len(), 2); // AIR (fill) + STONE
        for x in 0..32 {
            assert_eq!(s.get(x, 0, 0), STONE);
        }
    }

    #[test]
    fn semantic_equality_ignores_representation() {
        let mut a = Section::empty();
        let mut b = Section::empty();
        a.set(1, 2, 3, STONE);
        a.set(1, 2, 3, AIR); // a now has a bloated palette but identical content
        assert_eq!(a, b);
        b.set(0, 0, 0, GRASS);
        assert_ne!(a, b);
    }

    #[test]
    fn unpack_into_matches_get() {
        let mut s = Section::empty();
        s.set(0, 0, 0, GRASS);
        s.set(31, 31, 31, STONE);
        let mut flat = vec![AIR; SECTION_SIZE * SECTION_SIZE * SECTION_SIZE];
        s.unpack_into(&mut flat);
        assert_eq!(flat[0], GRASS);
        assert_eq!(flat[(31 * SECTION_SIZE + 31) * SECTION_SIZE + 31], STONE);
        assert_eq!(flat[1], AIR);
    }
}
```

- [ ] **Step 2: Run tests to verify the new ones fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml world::section`
Expected: FAIL — `uniform_block`, `voxel_data_bytes`, `palette_len`, `compact`, `unpack_into` not found.

- [ ] **Step 3: Implement the paletted section**

Replace everything above the tests module in `dabcraft/src/world/section.rs`:

```rust
use crate::world::block::{BlockId, AIR};

pub const SECTION_SIZE: usize = 32;
const VOLUME: usize = SECTION_SIZE * SECTION_SIZE * SECTION_SIZE;

/// Palette-compressed 32³ block storage (spec §4). `palette[0]` is the
/// uniform fill value; `bits == 0` means uniform (no voxel data at all).
/// Packed indices may span u64 word boundaries; VOLUME * bits is always a
/// multiple of 64, so a spanning entry's second word always exists.
#[derive(Clone, Debug)]
pub struct Section {
    palette: Vec<BlockId>,
    bits: u32,
    data: Vec<u64>,
}

fn bits_for(palette_len: usize) -> u32 {
    if palette_len <= 1 {
        0
    } else {
        usize::BITS - (palette_len - 1).leading_zeros()
    }
}

impl Section {
    pub fn empty() -> Self {
        Self { palette: vec![AIR], bits: 0, data: Vec::new() }
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < SECTION_SIZE && y < SECTION_SIZE && z < SECTION_SIZE);
        (y * SECTION_SIZE + z) * SECTION_SIZE + x
    }

    fn read_index(&self, voxel: usize) -> usize {
        let bit = voxel * self.bits as usize;
        let (word, off) = (bit / 64, bit % 64);
        let mask = (1u64 << self.bits) - 1;
        let mut v = self.data[word] >> off;
        if off + self.bits as usize > 64 {
            v |= self.data[word + 1] << (64 - off);
        }
        (v & mask) as usize
    }

    fn write_index(&mut self, voxel: usize, value: usize) {
        let bits = self.bits as usize;
        let bit = voxel * bits;
        let (word, off) = (bit / 64, bit % 64);
        let mask = (1u64 << bits) - 1;
        self.data[word] &= !(mask << off);
        self.data[word] |= (value as u64) << off;
        if off + bits > 64 {
            let spill = 64 - off;
            self.data[word + 1] &= !(mask >> spill);
            self.data[word + 1] |= (value as u64) >> spill;
        }
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockId {
        if self.bits == 0 {
            return self.palette[0];
        }
        self.palette[self.read_index(Self::index(x, y, z))]
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, block: BlockId) {
        let pi = match self.palette.iter().position(|&b| b == block) {
            Some(i) => i,
            None => {
                self.palette.push(block);
                let needed = bits_for(self.palette.len());
                if needed > self.bits {
                    self.repack(needed);
                }
                self.palette.len() - 1
            }
        };
        if self.bits == 0 {
            // Uniform and the value is already palette[0]: nothing to store.
            debug_assert_eq!(pi, 0);
            return;
        }
        self.write_index(Self::index(x, y, z), pi);
    }

    fn repack(&mut self, new_bits: u32) {
        let mut new_data = vec![0u64; VOLUME * new_bits as usize / 64];
        if self.bits > 0 {
            let old = std::mem::replace(&mut self.data, Vec::new());
            let old_bits = self.bits;
            self.data = new_data;
            // Re-read with old layout via a temporary view.
            let view = Section { palette: Vec::new(), bits: old_bits, data: old };
            self.bits = new_bits;
            for voxel in 0..VOLUME {
                let idx = view.read_index(voxel);
                self.write_index(voxel, idx);
            }
            return;
        }
        // Uniform → packed: all indices are 0 (palette[0]), already zeroed.
        self.bits = new_bits;
        self.data = new_data;
    }

    /// Out-of-bounds counts as air. Kept for the M1 naive mesher; the greedy
    /// mesher reads neighbors through PaddedSection instead.
    pub fn get_or_air(&self, x: i32, y: i32, z: i32) -> BlockId {
        let r = 0..SECTION_SIZE as i32;
        if r.contains(&x) && r.contains(&y) && r.contains(&z) {
            self.get(x as usize, y as usize, z as usize)
        } else {
            AIR
        }
    }

    /// Some(block) when every voxel holds the same block.
    pub fn uniform_block(&self) -> Option<BlockId> {
        if self.bits == 0 {
            return Some(self.palette[0]);
        }
        let first = self.read_index(0);
        (1..VOLUME).all(|v| self.read_index(v) == first).then(|| self.palette[first])
    }

    /// Bulk-decode all 32768 voxels into `out` (index = (y*32+z)*32+x).
    /// Hot path for padded-buffer fill; avoids 32768 bit-math get() calls
    /// staying generic — still one pass, but no per-call bounds checks.
    pub fn unpack_into(&self, out: &mut [BlockId]) {
        assert_eq!(out.len(), VOLUME);
        if self.bits == 0 {
            out.fill(self.palette[0]);
            return;
        }
        for (voxel, slot) in out.iter_mut().enumerate() {
            *slot = self.palette[self.read_index(voxel)];
        }
    }

    /// Rebuild the palette from live content, dropping orphaned entries and
    /// shrinking bit width. Call after worldgen finishes mutating a section.
    pub fn compact(&mut self) {
        let mut flat = vec![AIR; VOLUME];
        self.unpack_into(&mut flat);
        let fill = flat[0];
        let mut rebuilt = Section { palette: vec![fill], bits: 0, data: Vec::new() };
        for (voxel, &block) in flat.iter().enumerate() {
            if block != fill {
                let (x, z, y) = (voxel % 32, (voxel / 32) % 32, voxel / 1024);
                rebuilt.set(x, y, z, block);
            }
        }
        *self = rebuilt;
    }

    pub fn palette_len(&self) -> usize {
        self.palette.len()
    }

    pub fn voxel_data_bytes(&self) -> usize {
        self.data.len() * 8
    }
}

impl PartialEq for Section {
    /// Semantic equality: same blocks at same positions, regardless of
    /// palette order, orphaned entries, or bit width.
    fn eq(&self, other: &Self) -> bool {
        let mut a = vec![AIR; VOLUME];
        let mut b = vec![AIR; VOLUME];
        self.unpack_into(&mut a);
        other.unpack_into(&mut b);
        a == b
    }
}
```

Note for the implementer: in `compact()` the voxel→(x,y,z) inversion must match `index()`: `index = (y*32+z)*32+x`, so `x = voxel % 32`, `z = (voxel / 32) % 32`, `y = voxel / 1024` — the tuple order in the `let` above is `(x, z, y)` on purpose; pass them to `set(x, y, z, …)` in the right slots.

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: all PASS, including the M1 suite (naive mesher etc. still uses get/set).

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`
(If clippy demands `Vec::new()` → `vec![]` style changes, apply them.)

```bash
git add dabcraft/src/world/section.rs
git commit -m "feat: palette-compressed section storage with uniform fast path"
```

---

### Task 3: AO diagonal-flip bit in PackedQuad + shader corner rotation

The classic AO anisotropy fix: when `ao[0]+ao[2] > ao[1]+ao[3]`, the quad must triangulate along the other diagonal (spec §5). We pack a 1-bit flag into the free data1 bit 31; the vertex shader rotates the corner index by 1, which turns triangles (0,1,2)(0,2,3) into (1,2,3)(1,3,0) — same rectangle, flipped diagonal — with zero index-buffer changes.

**Files:**
- Modify: `dabcraft/src/mesh/quad.rs`
- Modify: `dabcraft/src/mesh/naive.rs` (add `flip: 0` to the emitted Quad)
- Modify: `dabcraft/assets/shaders/terrain.wgsl`

- [ ] **Step 1: Write the failing tests**

In `dabcraft/src/mesh/quad.rs` tests module, extend `data1_field_bit_positions` with one line and add a roundtrip:

```rust
// inside data1_field_bit_positions, after the texture assert:
assert_eq!(PackedQuad::pack(Quad { flip: 1, ..base }).data1, 1 << 31);
```

```rust
#[test]
fn flip_bit_roundtrips() {
    roundtrip(Quad {
        x: 3, y: 4, z: 5, face: 2, w: 2, h: 2,
        ao: [1, 2, 3, 0], skylight: 15, blocklight: 0, texture: 1, flip: 1,
    });
}
```

Every existing `Quad { ... }` literal in quad.rs tests gains `flip: 0` (the `base` quads too).

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test --manifest-path dabcraft/Cargo.toml mesh::quad`
Expected: FAIL — no field `flip`.

- [ ] **Step 3: Implement**

In `Quad`, after `texture`:

```rust
    pub texture: u32,  // 0..=1023, texture array layer
    pub flip: u32,     // 0|1: triangulate along the (w,0)-(0,h) diagonal (AO fix)
```

In `pack()`: add `debug_assert!(q.flip < 2);` and change the data1 line:

```rust
let data1 = (q.h - 1) | (ao << 5) | (q.skylight << 13) | (q.blocklight << 17)
    | (q.texture << 21) | (q.flip << 31);
```

In `unpack()`: add `flip: bits(self.data1, 31, 1),`.

In `dabcraft/src/mesh/naive.rs`, the emitted `Quad` literal gains `flip: 0`.

- [ ] **Step 4: Update the WGSL corner derivation**

In `dabcraft/assets/shaders/terrain.wgsl`, `vs_main` currently does `let corner = vi % 4u;`. Replace with:

```wgsl
    let flip = extractBits(quad.y, 31u, 1u);
    // AO diagonal flip: rotating the corner mapping by one turns the fixed
    // index pattern (0,1,2)(0,2,3) into triangles (1,2,3)(1,3,0) — the same
    // rectangle cut along the other diagonal. Positions and AO follow the
    // rotated corner, so geometry is identical and only the cut changes.
    let corner = (vi + flip) % 4u;
```

(`quad.y` is data1 — that naming already exists in the shader since quads are `vec2<u32>`.)

- [ ] **Step 5: Run tests + live smoke check**

Run: `cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`
Expected: PASS / clean.

Smoke (background, kill after ~5 s — macOS has no `timeout`):
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo run --release --manifest-path dabcraft/Cargo.toml &
APP_PID=$!; sleep 8; kill $APP_PID
```
Expected: no panics, no wgpu validation errors in the log; the M1 test island still renders (all flips are 0).

- [ ] **Step 6: Commit**

```bash
git add dabcraft/src/mesh/quad.rs dabcraft/src/mesh/naive.rs dabcraft/assets/shaders/terrain.wgsl
git commit -m "feat: add AO diagonal-flip bit to quad packing and shader"
```

---

### Task 4: PaddedSection — 34³ buffer with neighbor apron

The greedy mesher reads a single flat 34³ array: the 32³ section interior plus a 1-voxel apron from neighboring sections (spec §4). All apron cells are filled — faces, edges, AND corners — because AO samples diagonals. The interior is bulk-decoded (`unpack_into`); the apron uses a caller-provided closure with **section-local coordinates in −1..=32**.

**Files:**
- Create: `dabcraft/src/mesh/padded.rs`
- Modify: `dabcraft/src/mesh/mod.rs` (add `pub mod padded;`)

- [ ] **Step 1: Write the failing tests**

Create `dabcraft/src/mesh/padded.rs`:

```rust
use crate::world::block::{BlockId, AIR};
use crate::world::section::{Section, SECTION_SIZE};

/// Padded cube edge: 32 interior + 1 apron voxel on each side.
pub const PADDED: usize = SECTION_SIZE + 2;
const VOLUME: usize = PADDED * PADDED * PADDED;

/// Flat 34³ snapshot a mesh job works on. Padded coords are 0..34;
/// padded (x+1, y+1, z+1) == section-local (x, y, z).
pub struct PaddedSection {
    blocks: Box<[BlockId; VOLUME]>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{DIRT, GRASS, STONE};

    #[test]
    fn air_padded_is_all_air() {
        let p = PaddedSection::air();
        assert_eq!(p.get(0, 0, 0), AIR);
        assert_eq!(p.get(33, 33, 33), AIR);
    }

    #[test]
    fn interior_copies_section_at_plus_one_offset() {
        let mut s = Section::empty();
        s.set(0, 0, 0, GRASS);
        s.set(31, 31, 31, STONE);
        let p = PaddedSection::build(&s, |_, _, _| AIR);
        assert_eq!(p.get(1, 1, 1), GRASS);
        assert_eq!(p.get(32, 32, 32), STONE);
        assert_eq!(p.get(2, 1, 1), AIR);
    }

    #[test]
    fn apron_filled_from_neighbor_closure_in_local_coords() {
        let s = Section::empty();
        // Neighbor closure sees -1..=32; tag each apron cell by which
        // coordinate is out of range so we can verify faces/edges/corners.
        let p = PaddedSection::build(&s, |x, y, z| {
            let outside =
                u16::from(!(0..32).contains(&x)) + u16::from(!(0..32).contains(&y)) + u16::from(!(0..32).contains(&z));
            BlockId(outside)
        });
        assert_eq!(p.get(0, 5, 5), BlockId(1), "face apron");
        assert_eq!(p.get(33, 5, 5), BlockId(1));
        assert_eq!(p.get(0, 0, 5), BlockId(2), "edge apron");
        assert_eq!(p.get(33, 33, 33), BlockId(3), "corner apron");
        assert_eq!(p.get(5, 5, 5), AIR, "interior comes from the section, not the closure");
    }

    #[test]
    fn set_overrides_for_test_scenarios() {
        let mut p = PaddedSection::air();
        p.set(17, 3, 9, DIRT);
        assert_eq!(p.get(17, 3, 9), DIRT);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml mesh::padded`
Expected: compile FAIL — `air`, `build`, `get`, `set` missing. (Remember to add `pub mod padded;` to `dabcraft/src/mesh/mod.rs` first or the module won't be discovered.)

- [ ] **Step 3: Implement**

Insert between the struct and the tests module:

```rust
impl PaddedSection {
    pub fn air() -> Self {
        Self { blocks: Box::new([AIR; VOLUME]) }
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < PADDED && y < PADDED && z < PADDED);
        (y * PADDED + z) * PADDED + x
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockId {
        self.blocks[Self::index(x, y, z)]
    }

    /// Test scaffolding: build arbitrary voxel scenes without a Section.
    pub fn set(&mut self, x: usize, y: usize, z: usize, block: BlockId) {
        self.blocks[Self::index(x, y, z)] = block;
    }

    /// Interior from `center` (bulk-decoded once); apron cells — every padded
    /// cell with any coordinate 0 or 33 — from `neighbor`, which receives
    /// section-local coordinates in -1..=32 (exactly one or more out of range).
    pub fn build(center: &Section, neighbor: impl Fn(i32, i32, i32) -> BlockId) -> Self {
        let mut p = Self::air();
        let mut flat = vec![AIR; SECTION_SIZE * SECTION_SIZE * SECTION_SIZE];
        center.unpack_into(&mut flat);
        for y in 0..SECTION_SIZE {
            for z in 0..SECTION_SIZE {
                let row = (y * SECTION_SIZE + z) * SECTION_SIZE;
                let prow = Self::index(1, y + 1, z + 1);
                p.blocks[prow..prow + SECTION_SIZE].copy_from_slice(&flat[row..row + SECTION_SIZE]);
            }
        }
        for y in 0..PADDED {
            for z in 0..PADDED {
                for x in 0..PADDED {
                    if x == 0 || x == PADDED - 1 || y == 0 || y == PADDED - 1 || z == 0 || z == PADDED - 1 {
                        p.blocks[Self::index(x, y, z)] =
                            neighbor(x as i32 - 1, y as i32 - 1, z as i32 - 1);
                    }
                }
            }
        }
        p
    }
}
```

(The `copy_from_slice` row copy works because both layouts are x-contiguous with identical row composition.)

- [ ] **Step 4: Run tests, clippy, commit**

Run: `cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`
Expected: PASS / clean.

```bash
git add dabcraft/src/mesh/padded.rs dabcraft/src/mesh/mod.rs
git commit -m "feat: add 34-cubed padded section buffer with neighbor apron"
```

---

### Task 5: Binary greedy mesher — masks, face culling, plane merge (no AO yet)

The core of M2 (spec §5): occupancy bitmasks per axis, face culling with one shift+AND per 34-voxel column, greedy rectangle merge with `trailing_zeros`/`trailing_ones` sweeps. AO is Task 6; here every quad gets `ao: [3;4], flip: 0`. Light is constant in M2: `skylight: 15, blocklight: 0`.

**Files:**
- Create: `dabcraft/src/mesh/greedy.rs`
- Modify: `dabcraft/src/mesh/mod.rs` (add `pub mod greedy;`)

**Algorithm reference (binary greedy meshing, cgerikj/TanTanDev variant):**
1. **Occupancy:** 3 arrays of 34×34 `u64` columns. Bit `c` of `axis_cols[axis][i][j]` = solid at padded coordinate `c` along that axis. Layout: axis 0 bits=Y, `i`=z, `j`=x; axis 1 bits=X, `i`=y, `j`=z; axis 2 bits=Z, `i`=y, `j`=x.
2. **Face culling:** per interior column, `col & !(col >> 1)` = faces pointing **+axis** (solid at c, air at c+1); `col & !(col << 1)` = **−axis**. The apron bits make cross-section culling automatic. Then `(mask >> 1) as u32` drops the lower apron bit and truncates the upper one, leaving 32 interior bits (bit k = interior coordinate k).
3. **Plane binning:** each visible face voxel goes into a 32×32 bit plane keyed by `(face, slice, block)` — `plane[row] |= 1 << bit_col` with row=`i−1`, bit=`j−1`. Same-key faces are mergeable by construction, so the sweep needs zero comparisons.
4. **Greedy sweep per plane:** for each row, find runs with `trailing_zeros`/`trailing_ones`, extend across subsequent rows while `(plane[row+w] >> y) & run_mask == run_mask`, clearing consumed bits.

**Face/axis/plane mapping table (authoritative for this implementation — derived from the shipped FACE_U/FACE_V):**

| axis | bits | i (row) | j (bit) | +face | −face |
|---|---|---|---|---|---|
| 0 | Y | z | x | 2 (+Y) | 3 (−Y) |
| 1 | X | y | z | 0 (+X) | 1 (−X) |
| 2 | Z | y | x | 4 (+Z) | 5 (−Z) |

**Quad emission mapping** — a sweep result is (`slice` s = face-normal coord, `row` r = row start, `bit` b = bit start, `rw` = run across rows, `rb` = run along bits). Quad `w` spans the face's U axis, `h` spans V:

| face | quad x,y,z | quad w | quad h |
|---|---|---|---|
| 0 (+X) | (s, r, b) | rw (Y) | rb (Z) |
| 1 (−X) | (s, r, b) | rb (Z) | rw (Y) |
| 2 (+Y) | (b, s, r) | rw (Z) | rb (X) |
| 3 (−Y) | (b, s, r) | rb (X) | rw (Z) |
| 4 (+Z) | (b, r, s) | rb (X) | rw (Y) |
| 5 (−Z) | (b, r, s) | rw (Y) | rb (X) |

- [ ] **Step 1: Write the failing tests**

Create `dabcraft/src/mesh/greedy.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::padded::PaddedSection;
    use crate::mesh::quad::Quad;
    use crate::world::block::{DIRT, GRASS, STONE};

    fn mesh(p: &PaddedSection) -> Vec<Quad> {
        Mesher::new().mesh(p).iter().map(|pq| pq.unpack()).collect()
    }

    #[test]
    fn empty_section_emits_nothing() {
        assert!(mesh(&PaddedSection::air()).is_empty());
    }

    #[test]
    fn single_block_emits_six_unit_quads() {
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE); // padded coords; interior (5,5,5)
        let quads = mesh(&p);
        assert_eq!(quads.len(), 6);
        let mut faces: Vec<u32> = quads.iter().map(|q| q.face).collect();
        faces.sort_unstable();
        assert_eq!(faces, vec![0, 1, 2, 3, 4, 5]);
        for q in &quads {
            assert_eq!((q.w, q.h), (1, 1));
            assert_eq!((q.x, q.y, q.z), (5, 5, 5), "interior coords, face {}", q.face);
            assert_eq!(q.texture, STONE.0 as u32);
            assert_eq!(q.skylight, 15);
            assert_eq!(q.blocklight, 0);
        }
    }

    #[test]
    fn flat_slab_merges_to_one_quad_per_side() {
        // Full 32×32 floor, 1 thick: 6 quads total (naive M1 made 2176).
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for z in 1..=32 {
                p.set(x, 1, z, GRASS);
            }
        }
        let quads = mesh(&p);
        assert_eq!(quads.len(), 6);
        let top = quads.iter().find(|q| q.face == 2).unwrap();
        assert_eq!((top.w, top.h), (32, 32));
        assert_eq!((top.x, top.y, top.z), (0, 0, 0));
    }

    #[test]
    fn different_blocks_do_not_merge() {
        let mut p = PaddedSection::air();
        p.set(5, 5, 5, GRASS);
        p.set(6, 5, 5, DIRT);
        let quads = mesh(&p);
        let tops: Vec<_> = quads.iter().filter(|q| q.face == 2).collect();
        assert_eq!(tops.len(), 2);
        assert!(tops.iter().all(|q| (q.w, q.h) == (1, 1)));
    }

    #[test]
    fn solid_apron_culls_boundary_faces() {
        // Floor at interior y=0 with solid apron below: no -Y faces emitted.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for z in 1..=32 {
                p.set(x, 1, z, STONE); // interior floor
                p.set(x, 0, z, STONE); // apron below (neighbor section's top)
            }
        }
        let quads = mesh(&p);
        assert!(quads.iter().all(|q| q.face != 3), "bottom faces must be culled by the apron");
        assert_eq!(quads.iter().filter(|q| q.face == 2).count(), 1);
    }

    #[test]
    fn apron_never_emits_its_own_faces() {
        // Solid apron slab, empty interior: zero quads.
        let mut p = PaddedSection::air();
        for x in 0..34 {
            for z in 0..34 {
                p.set(x, 0, z, STONE);
            }
        }
        assert!(mesh(&p).is_empty());
    }

    #[test]
    fn interior_buried_voxels_emit_nothing() {
        // 3×3×3 solid cube: only the 6 outer 3×3 faces appear, 1 quad each.
        let mut p = PaddedSection::air();
        for x in 10..13 {
            for y in 10..13 {
                for z in 10..13 {
                    p.set(x, y, z, STONE);
                }
            }
        }
        let quads = mesh(&p);
        assert_eq!(quads.len(), 6);
        assert!(quads.iter().all(|q| (q.w, q.h) == (3, 3)));
    }

    #[test]
    fn l_shape_merges_greedily() {
        // Two-row L on the top face: rows z=5 (x 5..8) and z=6 (x 5..6).
        // Greedy sweep along bits(x) then rows(z): expect a 1×? split —
        // exact: top face quads cover 4 cells in 2 rectangles.
        let mut p = PaddedSection::air();
        for x in 6..9 {
            p.set(x, 6, 6, STONE);
        }
        p.set(6, 6, 7, STONE);
        let quads = mesh(&p);
        let tops: Vec<_> = quads.iter().filter(|q| q.face == 2).collect();
        let covered: u32 = tops.iter().map(|q| q.w * q.h).sum();
        assert_eq!(covered, 4, "top quads must tile the L exactly");
        assert_eq!(tops.len(), 2);
    }
}
```

- [ ] **Step 2: Run tests to verify compile failure**

Run: `cargo test --manifest-path dabcraft/Cargo.toml mesh::greedy`
Expected: FAIL — `Mesher` undefined. (Add `pub mod greedy;` to `mesh/mod.rs`.)

- [ ] **Step 3: Implement the mesher**

Above the tests in `dabcraft/src/mesh/greedy.rs`:

```rust
use std::collections::HashMap;

use crate::mesh::padded::{PaddedSection, PADDED};
use crate::mesh::quad::{PackedQuad, Quad};

const SIZE: usize = 32;

/// face for (axis, direction): FACE_OF[axis][0]=+dir, [1]=-dir.
const FACE_OF: [[u32; 2]; 3] = [[2, 3], [0, 1], [4, 5]];

/// Binary greedy mesher (spec §5). Reusable: `mesh` clears prior state.
/// M2: ao=[3;4], flip=0, skylight=15, blocklight=0 (AO lands in the next
/// task; flood-fill light in M4).
pub struct Mesher {
    /// Solidity columns. Bit c of axis_cols[axis][i][j] = solid at padded
    /// coord c along the axis. axis 0: bits=Y,i=z,j=x; 1: bits=X,i=y,j=z;
    /// 2: bits=Z,i=y,j=x.
    axis_cols: [[[u64; PADDED]; PADDED]; 3],
    /// (face,slice,block[,ao]) key → 32×32 face bit-plane.
    planes: HashMap<u64, [u32; SIZE]>,
    quads: Vec<PackedQuad>,
}

fn plane_key(face: u32, slice: u32, block: u16, ao_key: u32) -> u64 {
    block as u64 | (ao_key as u64) << 16 | (slice as u64) << 25 | (face as u64) << 31
}

impl Mesher {
    pub fn new() -> Self {
        Self {
            axis_cols: [[[0; PADDED]; PADDED]; 3],
            planes: HashMap::new(),
            quads: Vec::new(),
        }
    }

    pub fn mesh(&mut self, padded: &PaddedSection) -> Vec<PackedQuad> {
        self.axis_cols = [[[0; PADDED]; PADDED]; 3];
        self.planes.clear();
        self.quads.clear();
        self.build_axis_cols(padded);
        self.build_planes(padded);
        self.sweep_planes();
        std::mem::take(&mut self.quads)
    }

    fn build_axis_cols(&mut self, padded: &PaddedSection) {
        for y in 0..PADDED {
            for z in 0..PADDED {
                for x in 0..PADDED {
                    if padded.get(x, y, z).is_solid() {
                        self.axis_cols[0][z][x] |= 1 << y;
                        self.axis_cols[1][y][z] |= 1 << x;
                        self.axis_cols[2][y][x] |= 1 << z;
                    }
                }
            }
        }
    }

    fn build_planes(&mut self, padded: &PaddedSection) {
        for axis in 0..3 {
            for i in 1..=SIZE {
                for j in 1..=SIZE {
                    let col = self.axis_cols[axis][i][j];
                    // +dir: solid at c, air at c+1. -dir: air at c-1.
                    // >>1 drops the lower apron bit; the u32 cast truncates
                    // the upper one — only interior faces survive.
                    let pos = ((col & !(col >> 1)) >> 1) as u32;
                    let neg = ((col & !(col << 1)) >> 1) as u32;
                    for (dir, mut mask) in [(0usize, pos), (1usize, neg)] {
                        let face = FACE_OF[axis][dir];
                        while mask != 0 {
                            let c = mask.trailing_zeros();
                            mask &= mask - 1;
                            let (x, y, z) = match axis {
                                0 => (j, (c + 1) as usize, i),
                                1 => ((c + 1) as usize, i, j),
                                _ => (j, i, (c + 1) as usize),
                            };
                            let block = padded.get(x, y, z);
                            let ao_key = 0u32; // Task 6 replaces this
                            let key = plane_key(face, c, block.0, ao_key);
                            self.planes.entry(key).or_insert([0; SIZE])[i - 1] |= 1 << (j - 1);
                        }
                    }
                }
            }
        }
    }

    fn sweep_planes(&mut self) {
        let planes = std::mem::take(&mut self.planes);
        for (&key, plane) in planes.iter() {
            let block = (key & 0xFFFF) as u16;
            let ao_key = ((key >> 16) & 0x1FF) as u32;
            let slice = ((key >> 25) & 0x3F) as u32;
            let face = (key >> 31) as u32;
            sweep_plane(face, slice, block, ao_key, *plane, &mut self.quads);
        }
        self.planes = planes;
        self.planes.clear();
    }
}

impl Default for Mesher {
    fn default() -> Self {
        Self::new()
    }
}

/// Greedy rectangle decomposition of one 32×32 bit plane.
fn sweep_plane(
    face: u32,
    slice: u32,
    block: u16,
    ao_key: u32,
    mut plane: [u32; SIZE],
    out: &mut Vec<PackedQuad>,
) {
    for row in 0..SIZE {
        let mut b = 0u32;
        while b < SIZE as u32 {
            b += (plane[row] >> b).trailing_zeros();
            if b >= SIZE as u32 {
                break;
            }
            let rb = (plane[row] >> b).trailing_ones();
            let run_mask = u32::checked_shl(1, rb).map_or(!0, |v| v - 1);
            let mask = run_mask << b;
            let mut rw = 1usize;
            while row + rw < SIZE {
                if (plane[row + rw] >> b) & run_mask != run_mask {
                    break;
                }
                plane[row + rw] &= !mask;
                rw += 1;
            }
            emit(face, slice, row as u32, b, rw as u32, rb, block, ao_key, out);
            b += rb;
        }
    }
}

#[allow(clippy::too_many_arguments)] // internal plumbing of one algorithm step
fn emit(
    face: u32,
    slice: u32,
    row: u32,
    bit: u32,
    rw: u32,
    rb: u32,
    block: u16,
    ao_key: u32,
    out: &mut Vec<PackedQuad>,
) {
    // See the plan's face/plane mapping table; w spans U, h spans V.
    let ((x, y, z), w, h) = match face {
        0 => ((slice, row, bit), rw, rb),
        1 => ((slice, row, bit), rb, rw),
        2 => ((bit, slice, row), rw, rb),
        3 => ((bit, slice, row), rb, rw),
        4 => ((bit, row, slice), rb, rw),
        _ => ((bit, row, slice), rw, rb),
    };
    let ao = corner_ao(ao_key);
    let flip = u32::from(ao[0] + ao[2] > ao[1] + ao[3]);
    out.push(PackedQuad::pack(Quad {
        x, y, z, face, w, h,
        ao,
        skylight: 15,
        blocklight: 0,
        texture: block as u32,
        flip,
    }));
}

/// Task 6 gives this real content; until then every corner is fully lit.
fn corner_ao(_ao_key: u32) -> [u32; 4] {
    [3, 3, 3, 3]
}
```

Wiring check for the implementer: `plane_key` packs face into bits 31..34 of a u64 — decode in `sweep_planes` must match (`key >> 31`, no mask needed since nothing sits above face). The `(c + 1)` converts an interior bit index back to a padded coordinate.

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: all PASS (greedy suite + everything older).

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`

```bash
git add dabcraft/src/mesh/greedy.rs dabcraft/src/mesh/mod.rs
git commit -m "feat: binary greedy mesher with bitmask face culling and plane sweep"
```

---

### Task 6: Per-corner ambient occlusion in the greedy mesher

> **CORRECTION (found during implementation):** four test snippets below are flawed as written — helper blocks placed above the ground emit their own +Y faces, so (a) `ao_boundary_splits_merge` must count tops with `q.y == 5` only, and (b) `diagonal_only_neighbor_gives_corner_ao_two`, `fully_cornered_cell_gets_ao_zero`, and `anisotropic_ao_sets_flip_bit` must select the ground quad with `find(|q| q.face == 2 && (q.x, q.y, q.z) == (5, 5, 5))` instead of a bare `find(|q| q.face == 2)` (quad order is HashMap-nondeterministic). The shipped tests in `greedy.rs` are authoritative. The AO geometry tables are correct as written.

Real AO (spec §5): the classic 0–3 corner rule computed from the padded buffer, with merge correctness guaranteed by folding the face's 9-bit out-layer neighborhood pattern into the plane key — cells merge only when their entire AO-relevant neighborhood matches, so any cell's corner AO is valid for the whole merged quad. The diagonal flip from Task 3 activates here.

**Files:**
- Modify: `dabcraft/src/mesh/greedy.rs`

**AO geometry:** for a visible face of cell `p` (interior coords) with outward normal `n`, the "out layer" is `p + n`. Sample the 8 neighbors (plus the center, always air) of that out-layer cell at offsets `du·U + dv·V` for `(du, dv) ∈ {-1,0,1}²`, in **the face's own U/V axes** (same tables as the shader). Bit index in the key: `(du+1)*3 + (dv+1)`; bit set = solid. Corner AO (corner order `(0,0) (w,0) (w,h) (0,h)` in U/V space):

| corner | side1 bit | side2 bit | corner bit |
|---|---|---|---|
| 0 = (0,0) | 1 (−1,0) | 3 (0,−1) | 0 (−1,−1) |
| 1 = (w,0) | 7 (+1,0) | 3 (0,−1) | 6 (+1,−1) |
| 2 = (w,h) | 7 (+1,0) | 5 (0,+1) | 8 (+1,+1) |
| 3 = (0,h) | 1 (−1,0) | 5 (0,+1) | 2 (−1,+1) |

`ao = if side1 && side2 { 0 } else { 3 - (side1 + side2 + corner) }`.

Note: for a merged quad the far corners use the far cells' neighborhoods — identical to the local one because the key matched. Padded coords never go out of range: the normal step moves at most into the apron, and U/V steps from there stay within the 34³ cube (corners of the apron are filled — Task 4).

- [ ] **Step 1: Write the failing tests**

Add to the tests module of `dabcraft/src/mesh/greedy.rs`:

```rust
    #[test]
    fn isolated_block_has_full_ao() {
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        for q in mesh(&p) {
            assert_eq!(q.ao, [3, 3, 3, 3], "face {}", q.face);
            assert_eq!(q.flip, 0);
        }
    }

    #[test]
    fn wall_darkens_adjacent_top_corners() {
        // Ground block at interior (5,5,5); wall block at (6,6,5) — one step
        // +X and one up. The ground's top face (+Y): U=Z, V=X; the wall sits
        // at dv=+1 (side bit 5), darkening corners 2=(w,h) and 3=(0,h).
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE); // ground, padded
        p.set(7, 7, 6, STONE); // wall above-right, padded
        let quads = mesh(&p);
        let top = quads.iter().find(|q| q.face == 2 && (q.x, q.y, q.z) == (5, 5, 5)).unwrap();
        assert_eq!(top.ao, [3, 3, 2, 2]);
    }

    #[test]
    fn diagonal_only_neighbor_gives_corner_ao_two() {
        // Only a diagonal block above the top face: corner=1, sides=0 → ao 2.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(7, 7, 7, STONE); // diagonal: +U(z) and +V(x)? top face U=Z,V=X →
                               // offset (du=+1, dv=+1) relative to out-layer (6,7,6)
        let quads = mesh(&p);
        let top = quads.iter().find(|q| q.face == 2).unwrap();
        assert_eq!(top.ao, [3, 3, 2, 3]); // only corner 2=(w,h) darkened
    }

    #[test]
    fn fully_cornered_cell_gets_ao_zero() {
        // Two perpendicular walls above the ground block: side1 && side2 → 0.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);  // ground
        p.set(7, 7, 6, STONE);  // +V wall (x+1 at out-layer height)
        p.set(6, 7, 7, STONE);  // +U wall (z+1 at out-layer height)
        let top = mesh(&p).into_iter().find(|q| q.face == 2).unwrap();
        assert_eq!(top.ao[2], 0, "corner (w,h) boxed in by both walls");
    }

    #[test]
    fn ao_boundary_splits_merge() {
        // 2-block strip; a wall darkens only one cell's neighborhood → the
        // top face must NOT merge into a single quad.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(6, 6, 7, STONE);  // strip along z
        p.set(7, 7, 6, STONE);  // wall darkening only the first cell
        let quads = mesh(&p);
        let tops: Vec<_> = quads.iter().filter(|q| q.face == 2).collect();
        assert_eq!(tops.len(), 2, "AO difference must split the merge");
    }

    #[test]
    fn anisotropic_ao_sets_flip_bit() {
        // Build a top face where ao[0]+ao[2] > ao[1]+ao[3]: darken corners 1
        // and 3 (the (w,0)/(0,h) diagonal) via two opposite diagonal blocks.
        // Top face U=Z, V=X. Corner 1=(w,0): du=+1,dv=-1 → diagonal block at
        // out-layer +z,-x. Corner 3=(0,h): du=-1,dv=+1 → -z,+x.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(5, 7, 7, STONE); // (x-1, z+1) above: darkens corner 1
        p.set(7, 7, 5, STONE); // (x+1, z-1) above: darkens corner 3
        let top = mesh(&p).into_iter().find(|q| q.face == 2).unwrap();
        assert_eq!(top.ao, [3, 2, 3, 2]);
        assert_eq!(top.flip, 1, "3+3 > 2+2 must flip the diagonal");
    }

    #[test]
    fn slab_interior_still_merges_fully() {
        // AO keys are uniform across a flat slab interior — but edge cells
        // see the apron (air) the same way, so the whole top still merges.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for z in 1..=32 {
                p.set(x, 1, z, GRASS);
            }
        }
        let quads = mesh(&p);
        assert_eq!(quads.iter().filter(|q| q.face == 2).count(), 1);
    }
```

Sanity note for the test author: in `wall_darkens_adjacent_top_corners`, the wall at padded (7,7,6) sits at out-layer offset `du=0, dv=+1` (U=Z unchanged, V=X +1) → side bit 5 → darkens corners 2 and 3 by one each → `[3,3,2,2]`. In `diagonal_only_neighbor_gives_corner_ao_two`, padded (7,7,7) is `du=+1, dv=+1` → bit 8 → corner 2 only.

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test --manifest-path dabcraft/Cargo.toml mesh::greedy`
Expected: the new tests FAIL (every quad still has ao `[3,3,3,3]`, merges don't split).

- [ ] **Step 3: Implement AO**

In `dabcraft/src/mesh/greedy.rs`, add the integer face-axis tables (mirrors of the WGSL tables — keep in sync, the shipped terrain.wgsl is authoritative):

```rust
/// Integer mirrors of the shader's FACE tables (cross(U,V) = outward normal).
const FACE_N: [[i32; 3]; 6] =
    [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
const FACE_U: [[i32; 3]; 6] =
    [[0, 1, 0], [0, 0, 1], [0, 0, 1], [1, 0, 0], [1, 0, 0], [0, 1, 0]];
const FACE_V: [[i32; 3]; 6] =
    [[0, 0, 1], [0, 1, 0], [1, 0, 0], [0, 0, 1], [0, 1, 0], [1, 0, 0]];
```

Add the neighborhood sampler (padded coords in, 9-bit key out):

```rust
/// 9-bit solidity pattern of the out-layer 3×3 neighborhood of a face,
/// in the face's U/V axes. Bit (du+1)*3+(dv+1); (px,py,pz) are the padded
/// coords of the solid cell owning the face.
fn ao_neighborhood(padded: &PaddedSection, px: usize, py: usize, pz: usize, face: usize) -> u32 {
    let n = FACE_N[face];
    let (u, v) = (FACE_U[face], FACE_V[face]);
    let base = [px as i32 + n[0], py as i32 + n[1], pz as i32 + n[2]];
    let mut key = 0u32;
    for du in -1..=1i32 {
        for dv in -1..=1i32 {
            let q = [
                base[0] + du * u[0] + dv * v[0],
                base[1] + du * u[1] + dv * v[1],
                base[2] + du * u[2] + dv * v[2],
            ];
            // The normal step keeps exactly one coordinate at the apron rim;
            // U/V steps move the other two, which started in 1..=32. All
            // three therefore stay inside 0..34.
            if padded.get(q[0] as usize, q[1] as usize, q[2] as usize).is_solid() {
                key |= 1 << ((du + 1) * 3 + (dv + 1));
            }
        }
    }
    key
}
```

Replace the `corner_ao` stub:

```rust
/// Corner rule (spec §5): side1 && side2 → 0, else 3-(side1+side2+corner).
/// Corner order (0,0) (w,0) (w,h) (0,h) in face U/V space.
fn corner_ao(ao_key: u32) -> [u32; 4] {
    let bit = |i: u32| (ao_key >> i) & 1;
    let rule = |s1: u32, s2: u32, c: u32| {
        if s1 == 1 && s2 == 1 { 0 } else { 3 - (s1 + s2 + c) }
    };
    [
        rule(bit(1), bit(3), bit(0)),
        rule(bit(7), bit(3), bit(6)),
        rule(bit(7), bit(5), bit(8)),
        rule(bit(1), bit(5), bit(2)),
    ]
}
```

In `build_planes`, replace `let ao_key = 0u32; // Task 6 replaces this` with:

```rust
let ao_key = ao_neighborhood(padded, x, y, z, face as usize);
```

- [ ] **Step 4: Run the full suite**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: all PASS — including Task 5 tests (isolated geometry has all-air neighborhoods → ao stays `[3,3,3,3]`, merges unchanged).

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`

```bash
git add dabcraft/src/mesh/greedy.rs
git commit -m "feat: per-corner AO with merge-safe neighborhood keys and diagonal flip"
```

---

### Task 7: Worldgen — heightmap, biomes, column fill

Deterministic terrain from a 64-bit-seedable noise stack (spec §4): continentalness/erosion/peaks heightmap, temperature+humidity biome selection (6 biomes), sea level 64, per-biome surface blocks. Caves are Task 8, decoration Task 9 — this task generates solid terrain + water.

**Files:**
- Create: `dabcraft/src/world/gen.rs`
- Modify: `dabcraft/src/world/mod.rs` (add `pub mod gen;`)

- [ ] **Step 1: Write the failing tests**

Create `dabcraft/src/world/gen.rs` starting with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{AIR, STONE, WATER};

    #[test]
    fn same_seed_is_bit_identical() {
        let a = WorldGen::new(42).generate_column(3, -7);
        let b = WorldGen::new(42).generate_column(3, -7);
        assert_eq!(a.0.sections, b.0.sections);
        assert_eq!(a.1.len(), b.1.len());
    }

    #[test]
    fn different_seeds_differ() {
        let a = WorldGen::new(1).generate_column(0, 0);
        let b = WorldGen::new(2).generate_column(0, 0);
        assert_ne!(a.0.sections, b.0.sections, "two seeds producing identical terrain is wrong");
    }

    #[test]
    fn height_is_within_world_bounds() {
        let gen = WorldGen::new(1337);
        for &(x, z) in &[(0, 0), (1000, -2000), (-50000, 99999), (7, 13)] {
            let h = gen.height(x, z);
            assert!((4..=230).contains(&h), "height {h} at ({x},{z})");
        }
    }

    #[test]
    fn column_has_stone_below_surface_and_air_above() {
        let gen = WorldGen::new(1337);
        let (col, _) = gen.generate_column(0, 0);
        let h = gen.height(5, 5);
        let block_at = |y: i32| col.sections[(y / 32) as usize].get(5, (y % 32) as usize, 5);
        assert_eq!(block_at(1), STONE, "deep underground is stone");
        assert_ne!(block_at(h), AIR, "surface block exists at the heightmap value");
        if h >= SEA_LEVEL {
            assert_eq!(block_at(h + 1), AIR, "air above land surface");
        }
        assert_eq!(block_at(250), AIR, "top of world is air");
    }

    #[test]
    fn below_sea_level_terrain_is_flooded() {
        // Search a wide area for an ocean column; the seed guarantees oceans
        // exist somewhere in ±64 columns.
        let gen = WorldGen::new(1337);
        let mut found = false;
        'outer: for cx in -64..64 {
            for cz in -64..64 {
                let (wx, wz) = (cx * 32 + 16, cz * 32 + 16);
                let h = gen.height(wx, wz);
                if h < SEA_LEVEL - 2 {
                    let (col, _) = gen.generate_column(cx, cz);
                    let sea = SEA_LEVEL as usize;
                    let b = col.sections[sea / 32].get(16, sea % 32, 16);
                    assert_eq!(b, WATER, "cell above ocean floor at sea level must be water");
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "no ocean found in 128×128 columns — heightmap tuning is broken");
    }

    #[test]
    fn biome_matches_height_extremes() {
        let gen = WorldGen::new(1337);
        for cx in -32..32 {
            for cz in -32..32 {
                let (wx, wz) = (cx * 32, cz * 32);
                let h = gen.height(wx, wz);
                let biome = gen.biome(wx, wz);
                if h < SEA_LEVEL - 1 {
                    assert_eq!(biome, Biome::Ocean);
                }
                if h > 108 {
                    assert!(matches!(biome, Biome::Mountains | Biome::SnowyMountains));
                }
            }
        }
    }
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test --manifest-path dabcraft/Cargo.toml world::gen`
Expected: compile FAIL — `WorldGen` undefined. (Add `pub mod gen;` to `world/mod.rs`.)

- [ ] **Step 3: Implement**

Above the tests in `dabcraft/src/world/gen.rs`:

```rust
use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};

use crate::world::block::{
    BlockId, AIR, DIRT, GRASS, SAND, SNOW_GRASS, STONE, WATER,
};
use crate::world::section::Section;

pub const SEA_LEVEL: i32 = 64;
pub const COLUMN_SECTIONS: usize = 8;
pub const WORLD_HEIGHT: i32 = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Biome {
    Plains,
    Forest,
    Desert,
    Mountains,
    SnowyMountains,
    Ocean,
}

/// One generated 32×256×32 column: 8 stacked sections, bottom-up.
pub struct ColumnData {
    pub sections: Vec<Section>,
}

/// A block a structure wants to place outside (or inside) its own column.
/// `only_air`: don't overwrite terrain or other structures (used by leaves).
#[derive(Clone, Copy, Debug)]
pub struct StructureWrite {
    pub pos: glam::IVec3,
    pub block: BlockId,
    pub only_air: bool,
}

/// Deterministic worldgen (spec §4). FastNoiseLite is Copy: WorldGen is a
/// plain value cloned into every rayon job.
#[derive(Clone, Copy)]
pub struct WorldGen {
    continental: FastNoiseLite,
    erosion: FastNoiseLite,
    peaks: FastNoiseLite,
    temperature: FastNoiseLite,
    humidity: FastNoiseLite,
    cave_a: FastNoiseLite,
    cave_b: FastNoiseLite,
    cheese: FastNoiseLite,
    pub seed: i32,
}

fn noise(seed: i32, salt: i32, freq: f32, fractal: Option<FractalType>, octaves: i32) -> FastNoiseLite {
    let mut n = FastNoiseLite::with_seed(seed.wrapping_add(salt));
    n.set_noise_type(Some(NoiseType::OpenSimplex2));
    n.set_frequency(Some(freq));
    if let Some(f) = fractal {
        n.set_fractal_type(Some(f));
        n.set_fractal_octaves(Some(octaves));
    }
    n
}

impl WorldGen {
    pub fn new(seed: i32) -> Self {
        Self {
            continental: noise(seed, 1, 0.0011, Some(FractalType::FBm), 4),
            erosion: noise(seed, 2, 0.0019, Some(FractalType::FBm), 3),
            peaks: noise(seed, 3, 0.004, Some(FractalType::Ridged), 4),
            temperature: noise(seed, 4, 0.0007, Some(FractalType::FBm), 2),
            humidity: noise(seed, 5, 0.0009, Some(FractalType::FBm), 2),
            // Single-octave 3D noise: caves cost 2-3 samples per voxel and
            // octaves multiply that — tunnels don't need fractal detail.
            cave_a: noise(seed, 6, 0.012, None, 0),
            cave_b: noise(seed, 7, 0.012, None, 0),
            cheese: noise(seed, 8, 0.004, Some(FractalType::FBm), 2),
            seed,
        }
    }

    /// Terrain height (the y of the top solid block), 4..=230.
    pub fn height(&self, x: i32, z: i32) -> i32 {
        let (xf, zf) = (x as f32, z as f32);
        let c = self.continental.get_noise_2d(xf, zf); // -1..1
        let e = (self.erosion.get_noise_2d(xf, zf) + 1.0) * 0.5; // 0..1
        let p = (self.peaks.get_noise_2d(xf, zf) + 1.0) * 0.5; // 0..1
        let base = 64.0 + c * 36.0;
        // Peaks fade out toward (and below) the coast and under high erosion.
        let inland = ((c + 0.1) * 2.5).clamp(0.0, 1.0);
        let mountains = p * p * 90.0 * (1.0 - e * 0.8) * inland;
        (base + mountains).clamp(4.0, 230.0) as i32
    }

    pub fn biome(&self, x: i32, z: i32) -> Biome {
        self.biome_for(self.height(x, z), x, z)
    }

    fn biome_for(&self, height: i32, x: i32, z: i32) -> Biome {
        let t = self.temperature.get_noise_2d(x as f32, z as f32);
        let hu = self.humidity.get_noise_2d(x as f32, z as f32);
        if height < SEA_LEVEL - 1 {
            Biome::Ocean
        } else if height > 108 {
            if t < 0.0 { Biome::SnowyMountains } else { Biome::Mountains }
        } else if t > 0.35 && hu < 0.0 {
            Biome::Desert
        } else if hu > 0.05 {
            Biome::Forest
        } else {
            Biome::Plains
        }
    }

    fn surface_block(biome: Biome, height: i32) -> BlockId {
        match biome {
            Biome::Ocean => SAND,
            Biome::Desert => SAND,
            Biome::SnowyMountains => SNOW_GRASS,
            Biome::Mountains if height > 130 => STONE,
            // Beaches: any biome right at the waterline gets sand.
            _ if height <= SEA_LEVEL + 1 => SAND,
            _ => GRASS,
        }
    }

    fn subsurface_block(biome: Biome) -> BlockId {
        match biome {
            Biome::Desert | Biome::Ocean => SAND,
            _ => DIRT,
        }
    }

    /// Generate column (cx, cz). Returns the column plus structure writes
    /// that fell OUTSIDE it (trees crossing the border, Task 9).
    pub fn generate_column(&self, cx: i32, cz: i32) -> (ColumnData, Vec<StructureWrite>) {
        let mut sections: Vec<Section> = (0..COLUMN_SECTIONS).map(|_| Section::empty()).collect();
        for lx in 0..32usize {
            for lz in 0..32usize {
                let (wx, wz) = (cx * 32 + lx as i32, cz * 32 + lz as i32);
                let h = self.height(wx, wz);
                let biome = self.biome_for(h, wx, wz);
                for y in 0..=h {
                    let block = if y == h {
                        Self::surface_block(biome, h)
                    } else if y >= h - 3 {
                        Self::subsurface_block(biome)
                    } else if self.is_cave(wx, y, wz, h) {
                        continue; // carved: leave AIR
                    } else {
                        STONE
                    };
                    sections[(y / 32) as usize].set(lx, (y % 32) as usize, lz, block);
                }
                for y in (h + 1)..=SEA_LEVEL {
                    sections[(y / 32) as usize].set(lx, (y % 32) as usize, lz, WATER);
                }
            }
        }
        let writes = Vec::new(); // Task 9 fills this via decoration
        for s in &mut sections {
            s.compact();
        }
        (ColumnData { sections }, writes)
    }

    /// Task 8 implements carving; until then terrain is solid.
    fn is_cave(&self, _x: i32, _y: i32, _z: i32, _surface: i32) -> bool {
        false
    }
}
```

Note: `cave_a/b/cheese` fields are constructed now (so `new()` is final) but only used in Task 8 — silence the dead-field lint until then with `#[allow(dead_code)]` on the three fields if clippy complains (remove the allows in Task 8).

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: all PASS. The ocean-search test may take a couple of seconds (release-mode noise is fast; test profile uses opt-level 1 from the workspace profile — fine).

If `below_sea_level_terrain_is_flooded` fails to find an ocean, the heightmap constants are mistuned — fix the constants (e.g. raise `c * 36.0` toward `c * 40.0`), do NOT weaken the test.

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`

```bash
git add dabcraft/src/world/gen.rs dabcraft/src/world/mod.rs
git commit -m "feat: deterministic worldgen with heightmap, six biomes, and sea level"
```

---

### Task 8: Worldgen — caves

3D noise caves (spec §4): ridged "spaghetti" tunnels (two independent noises, carve where both are near zero — the classic intersection trick) plus low-frequency "cheese" rooms, attenuated near the surface (no carving above `surface - 8`, none below y 6).

**Files:**
- Modify: `dabcraft/src/world/gen.rs`

- [ ] **Step 1: Write the failing tests**

Add to the tests module:

```rust
    #[test]
    fn caves_exist_underground() {
        // Scan a deep slab of one column area for carved air. Range chosen
        // wide enough that absence means the thresholds are broken.
        let gen = WorldGen::new(1337);
        let mut carved = 0u32;
        let mut total = 0u32;
        for cx in 0..8 {
            let (col, _) = gen.generate_column(cx, 0);
            for x in 0..32 {
                for z in 0..32 {
                    let h = gen.height(cx * 32 + x as i32, z as i32);
                    for y in 8..(h - 8).max(8) {
                        total += 1;
                        if col.sections[(y / 32) as usize].get(x, (y % 32) as usize, z) == AIR {
                            carved += 1;
                        }
                    }
                }
            }
        }
        assert!(total > 0);
        let pct = carved as f32 / total as f32;
        assert!(pct > 0.005, "deep stone is {:.3}% carved — caves missing", pct * 100.0);
        assert!(pct < 0.35, "deep stone is {:.1}% carved — world is swiss cheese", pct * 100.0);
    }

    #[test]
    fn no_carving_near_surface_or_bedrock() {
        let gen = WorldGen::new(1337);
        for cx in 0..4 {
            let (col, _) = gen.generate_column(cx, 3);
            for x in 0..32 {
                for z in 0..32 {
                    let h = gen.height(cx * 32 + x as i32, 3 * 32 + z as i32);
                    for y in 0..6.min(h) {
                        assert_ne!(
                            col.sections[0].get(x, y as usize, z),
                            AIR,
                            "carved below y=6 at ({x},{y},{z})"
                        );
                    }
                    // The 4 blocks under the surface block are never carved
                    // (subsurface fill) and carving stops 8 below the surface,
                    // so the surface skin is always intact:
                    for y in (h - 7).max(0)..=h {
                        assert_ne!(
                            col.sections[(y / 32) as usize].get(x, (y % 32) as usize, z),
                            AIR,
                            "surface breach at ({x},{y},{z}), h={h}"
                        );
                    }
                }
            }
        }
    }
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test --manifest-path dabcraft/Cargo.toml world::gen`
Expected: `caves_exist_underground` FAILS (0% carved); the near-surface test passes vacuously.

- [ ] **Step 3: Implement carving**

Replace the `is_cave` stub (and remove any `#[allow(dead_code)]` from Task 7):

```rust
    /// Spaghetti tunnels: carve where two independent 3D noises are both
    /// near zero (their zero-surfaces intersect along winding 1D curves —
    /// inflated to tunnel radius by the threshold). Cheese rooms: rare
    /// low-frequency blobs. Attenuation: never within 8 blocks of the
    /// surface or below y=6, so the surface skin and world floor stay
    /// intact (spec §4 "attenuated near the surface").
    fn is_cave(&self, x: i32, y: i32, z: i32, surface: i32) -> bool {
        if y < 6 || y > surface - 8 {
            return false;
        }
        let (xf, zf) = (x as f32, z as f32);
        let yf = y as f32 * 1.7; // vertical squash → mostly-horizontal tunnels
        let a = self.cave_a.get_noise_3d(xf, yf, zf);
        let b = self.cave_b.get_noise_3d(xf, yf, zf);
        if a * a + b * b < 0.009 {
            return true;
        }
        self.cheese.get_noise_3d(xf, y as f32 * 2.0, zf) > 0.72
    }
```

- [ ] **Step 4: Run tests; tune thresholds only via constants**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: all PASS. If carved percentage is out of band, adjust `0.009` (tunnel radius) / `0.72` (room rarity) — keep the test bounds.

- [ ] **Step 5: Clippy + commit**

```bash
git add dabcraft/src/world/gen.rs
git commit -m "feat: spaghetti and cheese cave carving with surface attenuation"
```

---

### Task 9: Worldgen — trees, cacti, pending structure writes

Decoration (spec §4): oak trees (plains/forest), spruce (snowy mountains), cacti (desert), placed by deterministic per-position hashing. A structure is emitted as a list of `StructureWrite`s in **world coordinates**; writes inside the generating column are applied immediately, writes that cross the border are returned to the caller (the chunk manager queues them for the neighbor — Task 10).

**Files:**
- Create: `dabcraft/src/world/decor.rs`
- Modify: `dabcraft/src/world/gen.rs` (call decoration in `generate_column`)
- Modify: `dabcraft/src/world/mod.rs` (add `pub mod decor;`)

- [ ] **Step 1: Write the failing tests**

Create `dabcraft/src/world/decor.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{CACTUS, OAK_LEAVES, OAK_LOG, SPRUCE_LOG};
    use crate::world::gen::{Biome, WorldGen};

    #[test]
    fn hash_is_deterministic_and_position_sensitive() {
        assert_eq!(hash_pos(7, 100, -5, 1), hash_pos(7, 100, -5, 1));
        assert_ne!(hash_pos(7, 100, -5, 1), hash_pos(7, 101, -5, 1));
        assert_ne!(hash_pos(7, 100, -5, 1), hash_pos(8, 100, -5, 1));
        assert_ne!(hash_pos(7, 100, -5, 1), hash_pos(7, 100, -5, 2));
    }

    #[test]
    fn oak_tree_shape_is_well_formed() {
        let writes = oak_tree(glam::IVec3::new(10, 70, 10), 5);
        let logs: Vec<_> = writes.iter().filter(|w| w.block == OAK_LOG).collect();
        let leaves: Vec<_> = writes.iter().filter(|w| w.block == OAK_LEAVES).collect();
        assert_eq!(logs.len(), 5, "trunk height");
        assert!(logs.iter().all(|w| !w.only_air), "logs overwrite");
        assert!(leaves.iter().all(|w| w.only_air), "leaves never overwrite");
        assert!(leaves.len() > 20, "canopy exists");
        let top_log = logs.iter().map(|w| w.pos.y).max().unwrap();
        assert!(leaves.iter().any(|w| w.pos.y > top_log), "leaves above the trunk");
    }

    #[test]
    fn forest_columns_contain_trees() {
        // Find a forest column and assert at least one oak log was placed.
        let gen = WorldGen::new(1337);
        let mut found_forest = false;
        'outer: for cx in -48..48 {
            for cz in -48..48 {
                if gen.biome(cx * 32 + 16, cz * 32 + 16) != Biome::Forest {
                    continue;
                }
                found_forest = true;
                let (col, _) = gen.generate_column(cx, cz);
                let has_log = col.sections.iter().any(|s| {
                    let mut flat = vec![crate::world::block::AIR; 32 * 32 * 32];
                    s.unpack_into(&mut flat);
                    flat.contains(&OAK_LOG)
                });
                if has_log {
                    break 'outer;
                }
            }
        }
        assert!(found_forest, "no forest biome found — biome tuning broken");
        // Note: the FIRST forest column searched may legitimately have no
        // tree; the loop keeps searching until one has a log. If the loop
        // exhausts without breaking, this assert fires:
        let gen2 = WorldGen::new(1337);
        let any_tree = (-48..48).any(|cx| {
            (-48..48).any(|cz| {
                gen2.biome(cx * 32 + 16, cz * 32 + 16) == Biome::Forest && {
                    let (col, _) = gen2.generate_column(cx, cz);
                    col.sections.iter().any(|s| {
                        let mut flat = vec![crate::world::block::AIR; 32 * 32 * 32];
                        s.unpack_into(&mut flat);
                        flat.contains(&OAK_LOG)
                    })
                }
            })
        });
        assert!(any_tree, "no trees in any forest column");
    }

    #[test]
    fn border_trees_emit_out_of_column_writes() {
        // A canopy is 5 wide; a trunk within 2 blocks of the column edge must
        // spill leaves into the neighbor. Hunt for one and check the returned
        // writes are outside the column bounds.
        let gen = WorldGen::new(1337);
        for cx in -48..48 {
            for cz in -48..48 {
                let (_, writes) = gen.generate_column(cx, cz);
                if writes.is_empty() {
                    continue;
                }
                for w in &writes {
                    let in_x = (cx * 32..(cx + 1) * 32).contains(&w.pos.x);
                    let in_z = (cz * 32..(cz + 1) * 32).contains(&w.pos.z);
                    assert!(!(in_x && in_z), "returned write {:?} is inside its own column", w.pos);
                }
                return; // one spilling column is proof enough
            }
        }
        panic!("no border-crossing structure found in 96×96 columns");
    }

    #[test]
    fn spruce_and_cactus_shapes() {
        let spruce = spruce_tree(glam::IVec3::new(0, 100, 0), 6);
        assert_eq!(spruce.iter().filter(|w| w.block == SPRUCE_LOG).count(), 6);
        let cactus = cactus_plant(glam::IVec3::new(0, 65, 0), 2);
        assert_eq!(cactus.len(), 2);
        assert!(cactus.iter().all(|w| w.block == CACTUS));
    }
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test --manifest-path dabcraft/Cargo.toml world::decor`
Expected: compile FAIL — module functions missing.

- [ ] **Step 3: Implement decoration**

`dabcraft/src/world/decor.rs` above the tests:

```rust
use glam::IVec3;

use crate::world::block::{CACTUS, OAK_LEAVES, OAK_LOG, SPRUCE_LEAVES, SPRUCE_LOG};
use crate::world::gen::StructureWrite;

/// splitmix64 over (seed, x, z, salt): deterministic decoration decisions.
pub fn hash_pos(seed: i32, x: i32, z: i32, salt: u64) -> u64 {
    let mut v = (seed as u64)
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    v ^= v >> 30;
    v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    v ^= v >> 27;
    v = v.wrapping_mul(0x94D0_49BB_1331_11EB);
    v ^= v >> 31;
    v
}

/// Oak: straight trunk, 5×5 canopy (minus corners) on the top two trunk
/// levels, 3×3 above, plus a cross on top. `base` is the ground-surface
/// block; the trunk starts one above it.
pub fn oak_tree(base: IVec3, trunk: i32) -> Vec<StructureWrite> {
    let mut w = Vec::with_capacity(64);
    let top = base.y + trunk;
    for y in (base.y + 1)..=top {
        w.push(StructureWrite { pos: IVec3::new(base.x, y, base.z), block: OAK_LOG, only_air: false });
    }
    let mut leaf = |x: i32, y: i32, z: i32| {
        w.push(StructureWrite { pos: IVec3::new(x, y, z), block: OAK_LEAVES, only_air: true });
    };
    for y in (top - 1)..=top {
        for dx in -2..=2 {
            for dz in -2..=2 {
                if dx == 0 && dz == 0 && y <= top {
                    continue; // trunk occupies the center
                }
                if dx.abs() == 2 && dz.abs() == 2 {
                    continue; // clip corners
                }
                leaf(base.x + dx, y, base.z + dz);
            }
        }
    }
    for dx in -1..=1 {
        for dz in -1..=1 {
            if dx.abs() == 1 && dz.abs() == 1 {
                continue;
            }
            leaf(base.x + dx, top + 1, base.z + dz);
        }
    }
    w
}

/// Spruce: narrow — 3×3 (minus corners) rings on alternating upper trunk
/// levels and a single cap leaf.
pub fn spruce_tree(base: IVec3, trunk: i32) -> Vec<StructureWrite> {
    let mut w = Vec::with_capacity(32);
    let top = base.y + trunk;
    for y in (base.y + 1)..=top {
        w.push(StructureWrite { pos: IVec3::new(base.x, y, base.z), block: SPRUCE_LOG, only_air: false });
    }
    let mut leaf = |x: i32, y: i32, z: i32| {
        w.push(StructureWrite { pos: IVec3::new(x, y, z), block: SPRUCE_LEAVES, only_air: true });
    };
    for ring in 0..3 {
        let y = top - ring * 2 + 1;
        if y <= base.y + 2 {
            break;
        }
        for dx in -1..=1 {
            for dz in -1..=1 {
                if dx == 0 && dz == 0 {
                    continue;
                }
                if dx.abs() == 1 && dz.abs() == 1 && ring == 0 {
                    continue; // pointy top ring
                }
                leaf(base.x + dx, y, base.z + dz);
            }
        }
    }
    leaf(base.x, top + 2, base.z);
    w
}

pub fn cactus_plant(base: IVec3, height: i32) -> Vec<StructureWrite> {
    (1..=height)
        .map(|dy| StructureWrite {
            pos: IVec3::new(base.x, base.y + dy, base.z),
            block: CACTUS,
            only_air: false,
        })
        .collect()
}
```

- [ ] **Step 4: Wire decoration into `generate_column`**

In `dabcraft/src/world/gen.rs`:

Add imports: `use crate::world::decor::{cactus_plant, hash_pos, oak_tree, spruce_tree};` and `use glam::IVec3;`.

Replace `let writes = Vec::new(); // Task 9 fills this via decoration` and the compaction block at the end of `generate_column` with:

```rust
        // Decoration: deterministic per world (x,z); writes inside this
        // column apply now, the rest go back to the caller's pending queue.
        let mut outside = Vec::new();
        for lx in 0..32usize {
            for lz in 0..32usize {
                let (wx, wz) = (cx * 32 + lx as i32, cz * 32 + lz as i32);
                let h = self.height(wx, wz);
                let biome = self.biome_for(h, wx, wz);
                let surface = sections[(h / 32) as usize].get(lx, (h % 32) as usize, lz);
                if surface != Self::surface_block(biome, h) {
                    continue; // cave-adjacent oddity guard; surface must be intact
                }
                let roll = hash_pos(self.seed, wx, wz, 0xDEC0) % 1000;
                let structure = match biome {
                    Biome::Forest if roll < 60 => {
                        Some(oak_tree(IVec3::new(wx, h, wz), 4 + (hash_pos(self.seed, wx, wz, 1) % 3) as i32))
                    }
                    Biome::Plains if roll < 4 => {
                        Some(oak_tree(IVec3::new(wx, h, wz), 4 + (hash_pos(self.seed, wx, wz, 1) % 3) as i32))
                    }
                    Biome::SnowyMountains if roll < 25 && h < 140 => {
                        Some(spruce_tree(IVec3::new(wx, h, wz), 5 + (hash_pos(self.seed, wx, wz, 1) % 3) as i32))
                    }
                    Biome::Desert if roll < 12 => {
                        Some(cactus_plant(IVec3::new(wx, h, wz), 1 + (hash_pos(self.seed, wx, wz, 1) % 3) as i32))
                    }
                    _ => None,
                };
                let Some(structure) = structure else { continue };
                if biome != Biome::Desert && h <= SEA_LEVEL {
                    continue; // no trees on beaches/underwater
                }
                for write in structure {
                    let in_x = (cx * 32..(cx + 1) * 32).contains(&write.pos.x);
                    let in_z = (cz * 32..(cz + 1) * 32).contains(&write.pos.z);
                    if in_x && in_z {
                        apply_write(&mut sections, write);
                    } else {
                        outside.push(write);
                    }
                }
            }
        }
        for s in &mut sections {
            s.compact();
        }
        (ColumnData { sections }, outside)
```

Add the shared helper (pub — Task 10's ChunkMap reuses it):

```rust
/// Apply one structure write to a column's section stack (world y).
pub fn apply_write(sections: &mut [Section], write: StructureWrite) {
    if !(0..WORLD_HEIGHT).contains(&write.pos.y) {
        return;
    }
    let (lx, lz) = (write.pos.x.rem_euclid(32) as usize, write.pos.z.rem_euclid(32) as usize);
    let section = &mut sections[(write.pos.y / 32) as usize];
    let ly = (write.pos.y % 32) as usize;
    if write.only_air && section.get(lx, ly, lz) != AIR {
        return;
    }
    section.set(lx, ly, lz, write.block);
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: all PASS (decor suite, gen determinism still bit-identical — decoration is hash-driven, no shared state).

- [ ] **Step 6: Clippy + commit**

```bash
git add dabcraft/src/world/decor.rs dabcraft/src/world/gen.rs dabcraft/src/world/mod.rs
git commit -m "feat: tree and cactus decoration with cross-column pending writes"
```

---

### Task 10: ChunkMap, mesh neighborhoods, and the rayon/crossbeam job system

The streaming backbone (spec §3, §4): `ChunkMap` stores columns as `[Arc<Section>; 8]` (cheap to hand to mesh jobs), routes pending structure writes, and tracks dirty sections. `Jobs` fires `rayon::spawn` tasks and drains a crossbeam channel. `MeshNeighborhood` captures the 3×3×3 sections around a target so meshing runs entirely off-thread.

**Files:**
- Create: `dabcraft/src/world/chunks.rs`
- Create: `dabcraft/src/mesh/neighborhood.rs`
- Create: `dabcraft/src/world/jobs.rs`
- Modify: `dabcraft/src/world/mod.rs`, `dabcraft/src/mesh/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `dabcraft/src/world/chunks.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{AIR, OAK_LEAVES, STONE};
    use crate::world::gen::StructureWrite;

    fn empty_column_data() -> crate::world::gen::ColumnData {
        crate::world::gen::ColumnData {
            sections: (0..8).map(|_| crate::world::section::Section::empty()).collect(),
        }
    }

    #[test]
    fn block_to_column_floors_negative_coords() {
        assert_eq!(block_to_column(0, 0), ColumnPos { x: 0, z: 0 });
        assert_eq!(block_to_column(31, -1), ColumnPos { x: 0, z: -1 });
        assert_eq!(block_to_column(-1, -32), ColumnPos { x: -1, z: -1 });
        assert_eq!(block_to_column(-33, 64), ColumnPos { x: -2, z: 2 });
    }

    #[test]
    fn columns_in_radius_is_circular_and_distance_sorted() {
        let center = ColumnPos { x: 10, z: -5 };
        let cols = columns_in_radius(center, 3);
        assert!(cols.contains(&center));
        assert!(cols.contains(&ColumnPos { x: 13, z: -5 }), "cardinal edge included");
        assert!(!cols.contains(&ColumnPos { x: 13, z: -2 }), "corner outside the circle");
        assert_eq!(cols[0], center, "sorted by distance, center first");
        let d2 = |c: &ColumnPos| (c.x - 10).pow(2) + (c.z + 5).pow(2);
        assert!(cols.windows(2).all(|w| d2(&w[0]) <= d2(&w[1])));
    }

    #[test]
    fn insert_applies_queued_pending_writes() {
        let mut map = ChunkMap::default();
        // A neighbor generated earlier left a write for column (0,0):
        map.queue_writes(vec![StructureWrite {
            pos: glam::IVec3::new(5, 70, 5),
            block: OAK_LEAVES,
            only_air: true,
        }]);
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), Vec::new());
        let col = map.ready(ColumnPos { x: 0, z: 0 }).unwrap();
        assert_eq!(col.sections[2].get(5, 6, 5), OAK_LEAVES); // y 70 = section 2, local 6
    }

    #[test]
    fn insert_routes_writes_to_ready_columns_and_marks_dirty() {
        let mut map = ChunkMap::default();
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), Vec::new());
        // Clear dirty flags to observe the write-triggered dirtying:
        map.clear_all_dirty(ColumnPos { x: 0, z: 0 });
        // Column (1,0) generates and spills a write into (0,0):
        map.insert_generated(
            ColumnPos { x: 1, z: 0 },
            empty_column_data(),
            vec![StructureWrite { pos: glam::IVec3::new(31, 70, 5), block: STONE, only_air: false }],
        );
        let col = map.ready(ColumnPos { x: 0, z: 0 }).unwrap();
        assert_eq!(col.sections[2].get(31, 6, 5), STONE);
        assert!(col.dirty[2], "write must re-dirty the touched section");
    }

    #[test]
    fn writes_to_absent_columns_are_queued() {
        let mut map = ChunkMap::default();
        map.insert_generated(
            ColumnPos { x: 0, z: 0 },
            empty_column_data(),
            vec![StructureWrite { pos: glam::IVec3::new(40, 70, 5), block: STONE, only_air: false }],
        );
        // Column (1,0) doesn't exist yet; generating it later applies the write:
        map.insert_generated(ColumnPos { x: 1, z: 0 }, empty_column_data(), Vec::new());
        let col = map.ready(ColumnPos { x: 1, z: 0 }).unwrap();
        assert_eq!(col.sections[2].get(8, 6, 5), STONE); // 40 % 32 = 8
    }

    #[test]
    fn neighbors_ready_requires_full_3x3() {
        let mut map = ChunkMap::default();
        for dx in -1..=1 {
            for dz in -1..=1 {
                if (dx, dz) == (1, 1) {
                    continue;
                }
                map.insert_generated(ColumnPos { x: dx, z: dz }, empty_column_data(), Vec::new());
            }
        }
        assert!(!map.neighbors_ready(ColumnPos { x: 0, z: 0 }));
        map.insert_generated(ColumnPos { x: 1, z: 1 }, empty_column_data(), Vec::new());
        assert!(map.neighbors_ready(ColumnPos { x: 0, z: 0 }));
    }

    #[test]
    fn border_write_dirties_adjacent_sections_too() {
        // A write at a section's x=0 edge sits in the +X apron of the
        // neighbor section: that neighbor's mesh must be rebuilt as well.
        let mut map = ChunkMap::default();
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), Vec::new());
        map.insert_generated(ColumnPos { x: -1, z: 0 }, empty_column_data(), Vec::new());
        map.clear_all_dirty(ColumnPos { x: 0, z: 0 });
        map.clear_all_dirty(ColumnPos { x: -1, z: 0 });
        map.insert_generated(
            ColumnPos { x: 5, z: 5 }, // unrelated column whose gen spilled this write:
            empty_column_data(),
            vec![StructureWrite { pos: glam::IVec3::new(0, 64, 5), block: STONE, only_air: false }],
        );
        let col = map.ready(ColumnPos { x: 0, z: 0 }).unwrap();
        assert!(col.dirty[2], "own section dirty (y=64 → section 2)");
        assert!(col.dirty[1], "y=64 is section 2's bottom row → section 1 apron dirty");
        let west = map.ready(ColumnPos { x: -1, z: 0 }).unwrap();
        assert!(west.dirty[2], "x=0 is the west column's +X apron");
    }

    #[test]
    fn unload_removes_columns_outside_keep_radius() {
        let mut map = ChunkMap::default();
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), Vec::new());
        map.insert_generated(ColumnPos { x: 9, z: 0 }, empty_column_data(), Vec::new());
        let removed = map.unload_outside(ColumnPos { x: 0, z: 0 }, 5);
        assert_eq!(removed, vec![ColumnPos { x: 9, z: 0 }]);
        assert!(map.ready(ColumnPos { x: 9, z: 0 }).is_none());
        assert!(map.ready(ColumnPos { x: 0, z: 0 }).is_some());
    }
}
```

Create `dabcraft/src/mesh/neighborhood.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{AIR, STONE};
    use crate::world::section::Section;

    #[test]
    fn center_and_neighbor_lookups_resolve() {
        let mut center = Section::empty();
        center.set(0, 0, 0, STONE);
        let mut west = Section::empty();
        west.set(31, 5, 5, STONE);
        let mut hood = MeshNeighborhood::empty();
        hood.sections[MeshNeighborhood::index(0, 0, 0)] = Some(std::sync::Arc::new(center));
        hood.sections[MeshNeighborhood::index(-1, 0, 0)] = Some(std::sync::Arc::new(west));
        assert_eq!(hood.get(0, 0, 0), STONE);
        assert_eq!(hood.get(-1, 5, 5), STONE, "x=-1 reads the west neighbor's x=31");
        assert_eq!(hood.get(32, 5, 5), AIR, "missing east neighbor is air");
    }

    #[test]
    fn build_padded_wires_apron() {
        let mut center = Section::empty();
        center.set(0, 8, 8, STONE);
        let mut west = Section::empty();
        west.set(31, 8, 8, STONE);
        let mut hood = MeshNeighborhood::empty();
        hood.sections[MeshNeighborhood::index(0, 0, 0)] = Some(std::sync::Arc::new(center));
        hood.sections[MeshNeighborhood::index(-1, 0, 0)] = Some(std::sync::Arc::new(west));
        let padded = hood.build_padded();
        assert_eq!(padded.get(1, 9, 9), STONE, "interior");
        assert_eq!(padded.get(0, 9, 9), STONE, "west apron from the neighbor");
    }
}
```

Create `dabcraft/src/world/jobs.rs` with a smoke test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::chunks::ColumnPos;
    use crate::world::gen::WorldGen;

    #[test]
    fn gen_job_roundtrips_through_the_channel() {
        let mut jobs = Jobs::new();
        jobs.spawn_gen(WorldGen::new(7), ColumnPos { x: 0, z: 0 });
        assert_eq!(jobs.gen_in_flight, 1);
        // Worker pool latency: poll up to ~5 s.
        let mut results = Vec::new();
        for _ in 0..500 {
            results.extend(jobs.drain());
            if !results.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(results.len(), 1);
        assert_eq!(jobs.gen_in_flight, 0);
        match &results[0] {
            JobResult::Generated { pos, data, .. } => {
                assert_eq!(*pos, ColumnPos { x: 0, z: 0 });
                assert_eq!(data.sections.len(), 8);
            }
            other => panic!("expected Generated, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test --manifest-path dabcraft/Cargo.toml chunks`
Expected: compile FAIL. (Add `pub mod chunks; pub mod jobs;` to `world/mod.rs` and `pub mod neighborhood;` to `mesh/mod.rs`.)

- [ ] **Step 3: Implement `chunks.rs`**

```rust
use std::collections::HashMap;
use std::sync::Arc;

use crate::world::gen::{apply_write, ColumnData, StructureWrite, COLUMN_SECTIONS};
use crate::world::section::Section;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ColumnPos {
    pub x: i32,
    pub z: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SectionPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl SectionPos {
    pub fn origin(self) -> glam::IVec3 {
        glam::IVec3::new(self.x * 32, self.y * 32, self.z * 32)
    }

    pub fn column(self) -> ColumnPos {
        ColumnPos { x: self.x, z: self.z }
    }
}

pub fn block_to_column(wx: i32, wz: i32) -> ColumnPos {
    ColumnPos { x: wx.div_euclid(32), z: wz.div_euclid(32) }
}

/// Loaded column: 8 stacked sections behind Arcs (mesh jobs clone the Arcs,
/// never the data) + per-section dirty flags (dirty = needs (re)meshing).
pub struct Column {
    pub sections: [Arc<Section>; COLUMN_SECTIONS],
    pub dirty: [bool; COLUMN_SECTIONS],
}

enum Slot {
    /// Requested; a rayon job is generating it.
    Generating,
    Ready(Column),
}

#[derive(Default)]
pub struct ChunkMap {
    columns: HashMap<ColumnPos, Slot>,
    /// Structure writes waiting for their target column to generate.
    pending: HashMap<ColumnPos, Vec<StructureWrite>>,
}

impl ChunkMap {
    pub fn ready(&self, pos: ColumnPos) -> Option<&Column> {
        match self.columns.get(&pos) {
            Some(Slot::Ready(c)) => Some(c),
            _ => None,
        }
    }

    pub fn ready_mut(&mut self, pos: ColumnPos) -> Option<&mut Column> {
        match self.columns.get_mut(&pos) {
            Some(Slot::Ready(c)) => Some(c),
            _ => None,
        }
    }

    pub fn contains(&self, pos: ColumnPos) -> bool {
        self.columns.contains_key(&pos)
    }

    pub fn mark_generating(&mut self, pos: ColumnPos) {
        self.columns.insert(pos, Slot::Generating);
    }

    pub fn ready_count(&self) -> usize {
        self.columns.values().filter(|s| matches!(s, Slot::Ready(_))).count()
    }

    /// Test/streaming hook: queue writes for columns that don't exist yet.
    pub fn queue_writes(&mut self, writes: Vec<StructureWrite>) {
        for w in writes {
            self.pending
                .entry(block_to_column(w.pos.x, w.pos.z))
                .or_default()
                .push(w);
        }
    }

    /// Store a finished generation result: apply any writes other columns
    /// queued for it, then route ITS outside-writes to ready columns
    /// (applying + dirtying) or the pending queue.
    pub fn insert_generated(
        &mut self,
        pos: ColumnPos,
        mut data: ColumnData,
        outside_writes: Vec<StructureWrite>,
    ) {
        if let Some(queued) = self.pending.remove(&pos) {
            for w in queued {
                apply_write(&mut data.sections, w);
            }
        }
        let sections: [Arc<Section>; COLUMN_SECTIONS] = data
            .sections
            .into_iter()
            .map(Arc::new)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or_else(|_| unreachable!("generate_column always yields 8 sections"));
        self.columns.insert(
            pos,
            Slot::Ready(Column { sections, dirty: [true; COLUMN_SECTIONS] }),
        );
        for w in outside_writes {
            self.route_write(w);
        }
    }

    fn route_write(&mut self, w: StructureWrite) {
        let target = block_to_column(w.pos.x, w.pos.z);
        match self.columns.get_mut(&target) {
            Some(Slot::Ready(col)) => {
                if (0..256).contains(&w.pos.y) {
                    // Arc::make_mut: clone-on-write only if a mesh job still
                    // holds the old Arc; otherwise mutates in place.
                    let section = Arc::make_mut(&mut col.sections[(w.pos.y / 32) as usize]);
                    apply_write_to_section(section, w);
                    self.dirty_sections_touching(w.pos);
                }
            }
            _ => {
                self.pending.entry(target).or_default().push(w);
            }
        }
    }

    /// Mark dirty every section whose 34³ padded volume contains the world
    /// position — the owner plus any neighbor within 1 block across a border.
    pub fn dirty_sections_touching(&mut self, pos: glam::IVec3) {
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let p = pos + glam::IVec3::new(dx, dy, dz);
                    if !(0..256).contains(&p.y) {
                        continue;
                    }
                    let col = block_to_column(p.x, p.z);
                    let sy = (p.y / 32) as usize;
                    if let Some(Slot::Ready(c)) = self.columns.get_mut(&col) {
                        c.dirty[sy] = true;
                    }
                }
            }
        }
    }

    pub fn neighbors_ready(&self, pos: ColumnPos) -> bool {
        (-1..=1).all(|dx| {
            (-1..=1).all(|dz| {
                matches!(
                    self.columns.get(&ColumnPos { x: pos.x + dx, z: pos.z + dz }),
                    Some(Slot::Ready(_))
                )
            })
        })
    }

    /// Drop every column farther than `keep_radius` from `center`.
    /// Returns the removed positions so the renderer can free their meshes.
    pub fn unload_outside(&mut self, center: ColumnPos, keep_radius: i32) -> Vec<ColumnPos> {
        let r2 = keep_radius * keep_radius;
        let removed: Vec<ColumnPos> = self
            .columns
            .keys()
            .filter(|c| (c.x - center.x).pow(2) + (c.z - center.z).pow(2) > r2)
            .copied()
            .collect();
        for pos in &removed {
            self.columns.remove(pos);
        }
        removed
    }

    #[cfg(test)]
    pub fn clear_all_dirty(&mut self, pos: ColumnPos) {
        if let Some(Slot::Ready(c)) = self.columns.get_mut(&pos) {
            c.dirty = [false; COLUMN_SECTIONS];
        }
    }
}

/// All columns within `radius` (circular), nearest first.
pub fn columns_in_radius(center: ColumnPos, radius: i32) -> Vec<ColumnPos> {
    let r2 = radius * radius;
    let mut cols = Vec::new();
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if dx * dx + dz * dz <= r2 {
                cols.push(ColumnPos { x: center.x + dx, z: center.z + dz });
            }
        }
    }
    cols.sort_by_key(|c| (c.x - center.x).pow(2) + (c.z - center.z).pow(2));
    cols
}
```

`route_write` needs a per-section variant of Task 9's `apply_write`. In `gen.rs`, refactor `apply_write` into the pair below (the Task 9 tests keep passing — same behavior):

```rust
/// Apply one structure write to a column's section stack (world y).
pub fn apply_write(sections: &mut [Section], write: StructureWrite) {
    if !(0..WORLD_HEIGHT).contains(&write.pos.y) {
        return;
    }
    apply_write_to_section(&mut sections[(write.pos.y / 32) as usize], write);
}

/// Apply a write to the section that owns its y (caller picked it).
pub fn apply_write_to_section(section: &mut Section, write: StructureWrite) {
    let (lx, lz) = (write.pos.x.rem_euclid(32) as usize, write.pos.z.rem_euclid(32) as usize);
    let ly = (write.pos.y.rem_euclid(32)) as usize;
    if write.only_air && section.get(lx, ly, lz) != AIR {
        return;
    }
    section.set(lx, ly, lz, write.block);
}
```

`chunks.rs` imports both: `use crate::world::gen::{apply_write, apply_write_to_section, ColumnData, StructureWrite, COLUMN_SECTIONS};`

- [ ] **Step 4: Implement `neighborhood.rs`**

```rust
use std::sync::Arc;

use crate::mesh::padded::PaddedSection;
use crate::world::block::{BlockId, AIR};
use crate::world::section::Section;

/// The 3×3×3 sections around a mesh target, captured as Arc clones so the
/// mesh job reads consistent data with zero copying. Missing neighbors
/// (world top/bottom, unloaded horizontals — the scheduler prevents the
/// latter) read as air.
pub struct MeshNeighborhood {
    pub sections: [Option<Arc<Section>>; 27],
}

impl MeshNeighborhood {
    pub fn empty() -> Self {
        Self { sections: std::array::from_fn(|_| None) }
    }

    /// dx, dy, dz ∈ -1..=1.
    pub fn index(dx: i32, dy: i32, dz: i32) -> usize {
        debug_assert!((-1..=1).contains(&dx) && (-1..=1).contains(&dy) && (-1..=1).contains(&dz));
        ((dy + 1) * 9 + (dz + 1) * 3 + (dx + 1)) as usize
    }

    /// Section-local coords in -1..=32 (apron space) → block.
    pub fn get(&self, x: i32, y: i32, z: i32) -> BlockId {
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
        match &self.sections[Self::index(dx, dy, dz)] {
            Some(s) => s.get(lx, ly, lz),
            None => AIR,
        }
    }

    pub fn build_padded(&self) -> PaddedSection {
        let center = self.sections[Self::index(0, 0, 0)]
            .as_ref()
            .expect("mesh job scheduled without a center section");
        PaddedSection::build(center, |x, y, z| self.get(x, y, z))
    }
}
```

- [ ] **Step 5: Implement `jobs.rs`**

```rust
use crossbeam_channel::{Receiver, Sender};

use crate::mesh::greedy::Mesher;
use crate::mesh::neighborhood::MeshNeighborhood;
use crate::mesh::quad::PackedQuad;
use crate::world::chunks::{ColumnPos, SectionPos};
use crate::world::gen::{ColumnData, StructureWrite, WorldGen};

#[derive(Debug)]
pub enum JobResult {
    Generated { pos: ColumnPos, data: ColumnData, writes: Vec<StructureWrite> },
    Meshed { pos: SectionPos, quads: Vec<PackedQuad> },
}

/// Fire-and-forget rayon jobs with a crossbeam result channel (spec §3).
/// Job priority comes from submission order: the caller submits
/// nearest-first each frame and caps in-flight counts, so the pool's queue
/// stays short and camera-near work is never stuck behind a distant backlog.
pub struct Jobs {
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    pub gen_in_flight: usize,
    pub mesh_in_flight: usize,
}

impl Jobs {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self { tx, rx, gen_in_flight: 0, mesh_in_flight: 0 }
    }

    pub fn spawn_gen(&mut self, gen: WorldGen, pos: ColumnPos) {
        self.gen_in_flight += 1;
        let tx = self.tx.clone();
        rayon::spawn(move || {
            let (data, writes) = gen.generate_column(pos.x, pos.z);
            // Send fails only when the app is shutting down; fine to drop.
            let _ = tx.send(JobResult::Generated { pos, data, writes });
        });
    }

    pub fn spawn_mesh(&mut self, pos: SectionPos, hood: MeshNeighborhood) {
        self.mesh_in_flight += 1;
        let tx = self.tx.clone();
        rayon::spawn(move || {
            let padded = hood.build_padded();
            let quads = Mesher::new().mesh(&padded);
            let _ = tx.send(JobResult::Meshed { pos, quads });
        });
    }

    /// Non-blocking: everything that finished since the last call.
    pub fn drain(&mut self) -> Vec<JobResult> {
        let mut out = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
            match &r {
                JobResult::Generated { .. } => self.gen_in_flight -= 1,
                JobResult::Meshed { .. } => self.mesh_in_flight -= 1,
            }
            out.push(r);
        }
        out
    }
}

impl Default for Jobs {
    fn default() -> Self {
        Self::new()
    }
}
```

`ColumnData` needs `#[derive(Debug)]`? No — `JobResult` derives Debug, so add `#[derive(Debug)]` to `ColumnData` in gen.rs and `Section` already derives Debug (Task 2). Add it.

- [ ] **Step 6: Run tests**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: all PASS (the jobs test exercises a real rayon pool + channel roundtrip).

- [ ] **Step 7: Clippy + commit**

Run: `cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`
(Likely nits: `map(|a| Arc::make_mut(a))` closure simplification, `Default` impls — fix as clippy directs.)

```bash
git add dabcraft/src/world/chunks.rs dabcraft/src/world/jobs.rs dabcraft/src/world/mod.rs dabcraft/src/mesh/neighborhood.rs dabcraft/src/mesh/mod.rs dabcraft/src/world/gen.rs
git commit -m "feat: chunk map with pending writes plus rayon mesh and gen jobs"
```

---

### Task 11: Frustum culling

CPU frustum culling (spec §6): extract 6 planes from the view-projection matrix (Gribb–Hartmann), AABB test via the positive vertex. wgpu clip space is 0..1 depth, so the near plane is row 2 itself (not row3+row2).

**Files:**
- Create: `dabcraft/src/render/frustum.rs`
- Modify: `dabcraft/src/render/mod.rs` (add `pub mod frustum;`)

- [ ] **Step 1: Write the failing tests**

Create `dabcraft/src/render/frustum.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};

    fn camera_at_origin_looking_minus_z() -> Frustum {
        let proj = Mat4::perspective_rh(70f32.to_radians(), 16.0 / 9.0, 0.1, 500.0);
        let view = Mat4::look_to_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        Frustum::from_view_proj(proj * view)
    }

    #[test]
    fn box_in_front_is_visible() {
        let f = camera_at_origin_looking_minus_z();
        assert!(f.intersects_aabb(Vec3::new(-1.0, -1.0, -20.0), Vec3::new(1.0, 1.0, -18.0)));
    }

    #[test]
    fn box_behind_is_culled() {
        let f = camera_at_origin_looking_minus_z();
        assert!(!f.intersects_aabb(Vec3::new(-1.0, -1.0, 18.0), Vec3::new(1.0, 1.0, 20.0)));
    }

    #[test]
    fn box_beyond_far_plane_is_culled() {
        let f = camera_at_origin_looking_minus_z();
        assert!(!f.intersects_aabb(Vec3::new(-1.0, -1.0, -600.0), Vec3::new(1.0, 1.0, -590.0)));
    }

    #[test]
    fn box_far_to_the_side_is_culled() {
        let f = camera_at_origin_looking_minus_z();
        assert!(!f.intersects_aabb(Vec3::new(500.0, -1.0, -20.0), Vec3::new(502.0, 1.0, -18.0)));
    }

    #[test]
    fn box_straddling_a_plane_is_visible() {
        // Half in front of the camera, half behind: intersects ⇒ visible.
        let f = camera_at_origin_looking_minus_z();
        assert!(f.intersects_aabb(Vec3::new(-1.0, -1.0, -5.0), Vec3::new(1.0, 1.0, 5.0)));
    }

    #[test]
    fn enormous_box_containing_the_whole_frustum_is_visible() {
        let f = camera_at_origin_looking_minus_z();
        assert!(f.intersects_aabb(Vec3::splat(-10_000.0), Vec3::splat(10_000.0)));
    }
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test --manifest-path dabcraft/Cargo.toml render::frustum`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

```rust
use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};

/// View frustum as 6 inward-facing planes (xyz = normal, w = distance):
/// dot(n, p) + w >= 0 ⇔ p on the visible side.
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Gribb–Hartmann extraction. wgpu clip space: x,y ∈ -1..1, z ∈ 0..1,
    /// so near = row2 and far = row3 - row2.
    pub fn from_view_proj(m: Mat4) -> Self {
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        let mut planes = [
            r3 + r0, // left
            r3 - r0, // right
            r3 + r1, // bottom
            r3 - r1, // top
            r2,      // near (z >= 0)
            r3 - r2, // far  (z <= w)
        ];
        for p in &mut planes {
            let len = p.xyz().length();
            debug_assert!(len > 1e-6, "degenerate view-projection matrix");
            *p /= len;
        }
        Self { planes }
    }

    /// Positive-vertex test: for each plane, check the AABB corner farthest
    /// along the plane normal; if even that corner is outside, the whole box
    /// is. Conservative (a box outside a frustum *corner* can pass), which
    /// is correct for culling — never discards visible geometry.
    pub fn intersects_aabb(&self, min: Vec3, max: Vec3) -> bool {
        for p in &self.planes {
            let positive = Vec3::new(
                if p.x >= 0.0 { max.x } else { min.x },
                if p.y >= 0.0 { max.y } else { min.y },
                if p.z >= 0.0 { max.z } else { min.z },
            );
            if p.xyz().dot(positive) + p.w < 0.0 {
                return false;
            }
        }
        true
    }
}
```

- [ ] **Step 4: Run tests, clippy, commit**

Run: `cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`

```bash
git add dabcraft/src/render/frustum.rs dabcraft/src/render/mod.rs
git commit -m "feat: frustum plane extraction and AABB visibility test"
```

---

### Task 12: Arena allocator (slab + free-list)

Pure offset/length allocator for the big quad storage buffer (spec §5 "arena-allocated storage buffers with free-list"). First-fit over an offset-sorted free list with coalescing on free. Units are quads, not bytes — the renderer multiplies by 8.

**Files:**
- Create: `dabcraft/src/render/arena.rs`
- Modify: `dabcraft/src/render/mod.rs` (add `pub mod arena;`)

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_from_the_start() {
        let mut a = Arena::new(100);
        assert_eq!(a.alloc(10), Some(0));
        assert_eq!(a.alloc(20), Some(10));
        assert_eq!(a.used(), 30);
    }

    #[test]
    fn refuses_when_no_hole_fits() {
        let mut a = Arena::new(100);
        assert_eq!(a.alloc(60), Some(0));
        assert_eq!(a.alloc(50), None, "only 40 left");
        assert_eq!(a.used(), 60, "failed alloc must not change accounting");
    }

    #[test]
    fn freed_ranges_are_reused() {
        let mut a = Arena::new(100);
        let x = a.alloc(40).unwrap();
        a.alloc(40).unwrap();
        a.free(x, 40);
        assert_eq!(a.alloc(30), Some(0), "first-fit lands in the freed hole");
    }

    #[test]
    fn adjacent_frees_coalesce() {
        let mut a = Arena::new(100);
        let x = a.alloc(30).unwrap();
        let y = a.alloc(30).unwrap();
        let z = a.alloc(40).unwrap();
        assert_eq!((x, y, z), (0, 30, 60));
        a.free(x, 30);
        a.free(z, 40);
        a.free(y, 30); // middle free must merge with BOTH neighbors
        assert_eq!(a.used(), 0);
        assert_eq!(a.alloc(100), Some(0), "fully coalesced back to one range");
    }

    #[test]
    fn zero_len_alloc_is_rejected() {
        let mut a = Arena::new(100);
        assert_eq!(a.alloc(0), None);
    }

    #[test]
    fn exact_fit_consumes_the_hole() {
        let mut a = Arena::new(50);
        assert_eq!(a.alloc(50), Some(0));
        assert_eq!(a.alloc(1), None);
        a.free(0, 50);
        assert_eq!(a.alloc(50), Some(0));
    }
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test --manifest-path dabcraft/Cargo.toml render::arena`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

```rust
/// First-fit slab allocator over an abstract range of slots; the terrain
/// renderer maps slots to quads in one big storage buffer. Free list is
/// offset-sorted (offset, len) with full coalescing on free.
pub struct Arena {
    free: Vec<(u32, u32)>,
    capacity: u32,
    used: u32,
}

impl Arena {
    pub fn new(capacity: u32) -> Self {
        Self { free: vec![(0, capacity)], capacity, used: 0 }
    }

    pub fn alloc(&mut self, len: u32) -> Option<u32> {
        if len == 0 {
            return None;
        }
        let i = self.free.iter().position(|&(_, l)| l >= len)?;
        let (off, l) = self.free[i];
        if l == len {
            self.free.remove(i);
        } else {
            self.free[i] = (off + len, l - len);
        }
        self.used += len;
        Some(off)
    }

    /// Free a range previously returned by `alloc` (same len).
    pub fn free(&mut self, offset: u32, len: u32) {
        debug_assert!(offset + len <= self.capacity);
        let i = self.free.partition_point(|&(o, _)| o < offset);
        self.free.insert(i, (offset, len));
        self.used -= len;
        // Coalesce with the next range, then with the previous one.
        if i + 1 < self.free.len() && self.free[i].0 + self.free[i].1 == self.free[i + 1].0 {
            self.free[i].1 += self.free[i + 1].1;
            self.free.remove(i + 1);
        }
        if i > 0 && self.free[i - 1].0 + self.free[i - 1].1 == self.free[i].0 {
            self.free[i - 1].1 += self.free[i].1;
            self.free.remove(i);
        }
    }

    pub fn used(&self) -> u32 {
        self.used
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }
}
```

- [ ] **Step 4: Run tests, clippy, commit**

Run: `cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`

```bash
git add dabcraft/src/render/arena.rs dabcraft/src/render/mod.rs
git commit -m "feat: first-fit arena allocator with coalescing free list"
```

---

### Task 13: TerrainRenderer rework — arena buffers, section slots, indirect draws

The renderer becomes section-granular (spec §5, §6): one 32 MB quad arena (storage), a per-slot section-origin buffer (storage, addressed by `instance_index`), an indirect-args buffer, a static shared index buffer, and one `draw_indexed_indirect` per visible section with `first_instance = slot`. Backface culling turns ON (M1 fixed the winding for exactly this moment). To keep the build playable, app.rs is minimally adapted: the M1 test island goes through the new path (upload one section, frustum-cull, indirect-draw) until Task 14 replaces it with the streamed world.

**Verified wgpu 29 facts this task relies on:**
- `wgpu::util::DrawIndexedIndirectArgs { index_count, instance_count, first_index, base_vertex: i32, first_instance }` — 20 bytes, implements `Pod`, has `.as_bytes()`; `bytemuck::cast_slice` works on a `&[DrawIndexedIndirectArgs]`.
- `RenderPass::draw_indexed_indirect(&buffer, offset)`; offset must be 4-aligned (20-byte stride is fine).
- In an indexed draw, WGSL `@builtin(vertex_index)` = index-buffer value + `base_vertex`; `@builtin(instance_index)` starts at `first_instance`.
- Non-zero `first_instance` in INDIRECT draws **requires `Features::INDIRECT_FIRST_INSTANCE`** — without it the draw is silently skipped. Supported on all Apple Silicon Metal.
- `Queue::write_buffer` needs 4-aligned offset and length (PackedQuad = 8 B, SectionInfo = 16 B, args = 20 B — all fine).

**Files:**
- Modify: `dabcraft/src/render/gpu.rs`
- Rewrite: `dabcraft/src/render/terrain.rs`
- Rewrite: `dabcraft/assets/shaders/terrain.wgsl`
- Modify: `dabcraft/src/app.rs` (minimal adaptation)

- [ ] **Step 1: Require INDIRECT_FIRST_INSTANCE in gpu.rs**

In `Gpu::new`, where `required_features` is assembled (currently conditional `TIMESTAMP_QUERY`), add:

```rust
assert!(
    adapter.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
    "dabcraft requires INDIRECT_FIRST_INSTANCE (any Apple Silicon Metal device has it)"
);
required_features |= wgpu::Features::INDIRECT_FIRST_INSTANCE;
```

(Adapt to the actual local variable shape in gpu.rs — the M1 code builds the features value before `request_device`; extend that expression. Without this feature every indirect draw with a non-zero slot would be silently dropped.)

- [ ] **Step 2: Write the failing test for draw-arg construction**

The GPU plumbing isn't unit-testable, but the arg math is. In the new `dabcraft/src/render/terrain.rs`, tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_args_encode_arena_offset_and_slot() {
        let args = section_draw_args(1000, 24, 7);
        assert_eq!(args.index_count, 24 * 6, "6 indices per quad");
        assert_eq!(args.instance_count, 1);
        assert_eq!(args.first_index, 0);
        assert_eq!(args.base_vertex, 4000, "4 vertices per quad, offset in quads");
        assert_eq!(args.first_instance, 7, "slot rides in first_instance");
    }

    #[test]
    fn packed_args_are_20_bytes() {
        assert_eq!(std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>(), 20);
    }
}
```

Run: `cargo test --manifest-path dabcraft/Cargo.toml render::terrain` → FAIL (`section_draw_args` missing).

- [ ] **Step 3: Rewrite terrain.rs**

```rust
use std::collections::HashMap;

use crate::mesh::quad::{build_quad_indices, PackedQuad};
use crate::render::arena::Arena;
use crate::render::depth::DEPTH_FORMAT;
use crate::render::frustum::Frustum;
use crate::world::chunks::SectionPos;

/// Arena capacity in quads: 4M × 8 B = 32 MiB (well under the 128 MiB
/// default max storage binding). Greedy-meshed surface terrain at 12-column
/// radius measures in the hundreds of thousands of quads; 4M is headroom.
const QUAD_CAPACITY: u32 = 4 << 20;
/// Max resident sections (slots). Load diameter 27 → ~553 columns × 8 = 4424.
const MAX_SECTIONS: u32 = 8192;
/// Static index buffer covers the worst single section. The theoretical max
/// (3D checkerboard) is 32³/2 × 6 = 98 304 quads; 131 072 gives margin.
const MAX_QUADS_PER_SECTION: u32 = 1 << 17;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SectionInfo {
    origin: [i32; 4], // xyz world origin; w padding
}

struct SectionEntry {
    slot: u32,
    offset: u32, // in quads
    len: u32,    // in quads
}

pub struct DrawStats {
    pub resident_sections: u32,
    pub visible_sections: u32,
    pub drawn_quads: u32,
}

/// Builds the indirect args for one section: quads at `offset` (arena slots),
/// section data at `slot`. base_vertex shifts vertex_index by 4·offset so
/// `vi / 4` lands on the right arena quad; first_instance carries the slot
/// to the shader as instance_index (requires INDIRECT_FIRST_INSTANCE).
fn section_draw_args(offset: u32, len: u32, slot: u32) -> wgpu::util::DrawIndexedIndirectArgs {
    wgpu::util::DrawIndexedIndirectArgs {
        index_count: len * 6,
        instance_count: 1,
        first_index: 0,
        base_vertex: (offset * 4) as i32,
        first_instance: slot,
    }
}

pub struct TerrainRenderer {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_layout: wgpu::BindGroupLayout,
    quads_layout: wgpu::BindGroupLayout,
    quads_bind_group: wgpu::BindGroup,
    // The bind group references quads_buffer + section_info_buffer; both are
    // owned fields, so they outlive it by construction (M1's Option dance is
    // gone — all buffers are fixed-size and created once).
    quads_buffer: wgpu::Buffer,
    section_info_buffer: wgpu::Buffer,
    indirect_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    arena: Arena,
    entries: HashMap<SectionPos, SectionEntry>,
    free_slots: Vec<u32>,
    visible_count: u32,
    surface_format: wgpu::TextureFormat,
}

impl TerrainRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat, shader_source: &str) -> Self {
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let storage_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let quads_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quads+sections"),
            entries: &[storage_entry(0), storage_entry(1)],
        });

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });

        let quads_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad arena"),
            size: QUAD_CAPACITY as u64 * 8,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let section_info_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("section info"),
            size: MAX_SECTIONS as u64 * std::mem::size_of::<SectionInfo>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let indirect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect args"),
            size: MAX_SECTIONS as u64 * 20,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quads_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quads+sections"),
            layout: &quads_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: quads_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: section_info_buffer.as_entire_binding() },
            ],
        });

        let indices = build_quad_indices(MAX_QUADS_PER_SECTION);
        let index_buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("shared quad indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            },
        );

        let pipeline =
            Self::build_pipeline(device, surface_format, &camera_layout, &quads_layout, shader_source);

        Self {
            pipeline,
            camera_buffer,
            camera_bind_group,
            camera_layout,
            quads_layout,
            quads_bind_group,
            quads_buffer,
            section_info_buffer,
            indirect_buffer,
            index_buffer,
            arena: Arena::new(QUAD_CAPACITY),
            entries: HashMap::new(),
            free_slots: (0..MAX_SECTIONS).rev().collect(),
            visible_count: 0,
            surface_format,
        }
    }

    fn build_pipeline(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(camera_layout), Some(quads_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[], // vertex pulling: no vertex buffers
            },
            primitive: wgpu::PrimitiveState {
                // M1 verified CCW outward winding; greedy quads follow the
                // same FACE_U/FACE_V tables, so back faces are safe to cull.
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Replace the pipeline from new shader source (hot-reload).
    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.pipeline = Self::build_pipeline(
            device, self.surface_format, &self.camera_layout, &self.quads_layout, shader_source,
        );
    }

    /// Upload (or replace) one section's quads. Empty quads = section is
    /// resident but draws nothing (fully enclosed / all air).
    pub fn upload_section(&mut self, queue: &wgpu::Queue, pos: SectionPos, quads: &[PackedQuad]) {
        self.remove_section(pos);
        if quads.is_empty() {
            return;
        }
        if quads.len() as u32 > MAX_QUADS_PER_SECTION {
            // Unreachable for real terrain (worst case 98k); guard anyway.
            log::error!("section {pos:?} exceeds MAX_QUADS_PER_SECTION ({})", quads.len());
            return;
        }
        let Some(offset) = self.arena.alloc(quads.len() as u32) else {
            log::warn!("quad arena full; section {pos:?} not uploaded");
            return;
        };
        let Some(slot) = self.free_slots.pop() else {
            self.arena.free(offset, quads.len() as u32);
            log::warn!("section slots exhausted; section {pos:?} not uploaded");
            return;
        };
        queue.write_buffer(&self.quads_buffer, offset as u64 * 8, bytemuck::cast_slice(quads));
        let o = pos.origin();
        let info = SectionInfo { origin: [o.x, o.y, o.z, 0] };
        queue.write_buffer(
            &self.section_info_buffer,
            slot as u64 * std::mem::size_of::<SectionInfo>() as u64,
            bytemuck::bytes_of(&info),
        );
        self.entries.insert(pos, SectionEntry { slot, offset, len: quads.len() as u32 });
    }

    pub fn remove_section(&mut self, pos: SectionPos) {
        if let Some(e) = self.entries.remove(&pos) {
            self.arena.free(e.offset, e.len);
            self.free_slots.push(e.slot);
        }
    }

    pub fn write_camera(&self, queue: &wgpu::Queue, view_proj: glam::Mat4) {
        let uniform = CameraUniform { view_proj: view_proj.to_cols_array_2d() };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Frustum-cull resident sections and write this frame's indirect args.
    /// Call BEFORE the render pass; `draw` then replays `visible_count`
    /// indirect draws. write_buffer data lands before any subsequently
    /// submitted command buffer, so ordering is safe.
    pub fn prepare(&mut self, queue: &wgpu::Queue, frustum: &Frustum) -> DrawStats {
        let mut args: Vec<wgpu::util::DrawIndexedIndirectArgs> = Vec::with_capacity(self.entries.len());
        let mut drawn_quads = 0u32;
        for (pos, e) in &self.entries {
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
        }
    }

    pub fn arena_usage(&self) -> (u32, u32) {
        (self.arena.used(), self.arena.capacity())
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass<'_>) {
        if self.visible_count == 0 {
            return;
        }
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.camera_bind_group, &[]);
        rpass.set_bind_group(1, &self.quads_bind_group, &[]);
        rpass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for i in 0..self.visible_count {
            rpass.draw_indexed_indirect(&self.indirect_buffer, i as u64 * 20);
        }
    }
}
```

- [ ] **Step 4: Rewrite terrain.wgsl**

```wgsl
struct CameraUniform {
    view_proj: mat4x4<f32>,
};

struct SectionInfo {
    origin: vec4<i32>, // xyz = section world origin, w unused
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
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
// Minecraft-style face shading: +X, -X, +Y(top), -Y(bottom), +Z, -Z.
const FACE_SHADE = array<f32, 6>(0.8, 0.8, 1.0, 0.5, 0.6, 0.6);

// M2 block palette, indexed by the quad's texture field (= block id).
// Procedural textures replace this in M6.
const PALETTE = array<vec3<f32>, 12>(
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
);

// Corner order matches PackedQuad ao order: (0,0) (w,0) (w,h) (0,h).
const CORNER_UV = array<vec2<f32>, 4>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> VsOut {
    // base_vertex (4 × arena offset) is already folded into vi, so vi/4 is
    // the arena-global quad index; first_instance carries the section slot.
    let quad = quads[vi / 4u];
    let flip = extractBits(quad.y, 31u, 1u);
    // AO diagonal flip: rotating the corner mapping by one turns the fixed
    // index pattern (0,1,2)(0,2,3) into triangles (1,2,3)(1,3,0) — the same
    // rectangle cut along the other diagonal.
    let corner = (vi + flip) % 4u;

    let x = f32(extractBits(quad.x, 0u, 6u));
    let y = f32(extractBits(quad.x, 6u, 6u));
    let z = f32(extractBits(quad.x, 12u, 6u));
    let face = extractBits(quad.x, 18u, 3u);
    let w = f32(extractBits(quad.x, 21u, 5u) + 1u);
    let h = f32(extractBits(quad.y, 0u, 5u) + 1u);
    let ao = f32(extractBits(quad.y, 5u + corner * 2u, 2u));
    let skylight = f32(extractBits(quad.y, 13u, 4u));
    let tex = extractBits(quad.y, 21u, 10u);

    let uv = CORNER_UV[corner];
    let local = vec3(x, y, z) + FACE_ORIGIN[face] + FACE_U[face] * uv.x * w + FACE_V[face] * uv.y * h;
    let world = vec3<f32>(sections[slot].origin.xyz) + local;

    var out: VsOut;
    out.clip = camera.view_proj * vec4(world, 1.0);
    let light = (skylight / 15.0) * FACE_SHADE[face] * mix(0.4, 1.0, ao / 3.0);
    out.color = PALETTE[min(tex, 11u)] * light;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
```

- [ ] **Step 5: Minimal app.rs adaptation (full rework is Task 14)**

In `resumed()`, replace the M1 upload:

```rust
        let mut terrain = TerrainRenderer::new(&gpu.device, gpu.config.format, &shader_source);
        let quads = crate::mesh::naive::mesh_naive(&build_test_section());
        self.quad_count = quads.len() as u32;
        terrain.upload_section(&gpu.queue, crate::world::chunks::SectionPos { x: 0, y: 0, z: 0 }, &quads);
        self.terrain = Some(terrain);
```

In `render()`, after `write_camera` and before `gpu.acquire()`, prepare the indirect args (terrain needs `as_mut` now):

```rust
        let terrain = self.terrain.as_mut();
        let Some(depth_view_ref) = self.depth_view.as_ref() else { return };
        let Some(gpu) = self.gpu.as_mut() else { return };

        if let Some(terrain) = terrain {
            let aspect = gpu.config.width as f32 / gpu.config.height as f32;
            let view_proj = self.camera.view_proj(aspect);
            terrain.write_camera(&gpu.queue, view_proj);
            let frustum = crate::render::frustum::Frustum::from_view_proj(view_proj);
            terrain.prepare(&gpu.queue, &frustum);
        }
```

…and inside the pass the existing `terrain.draw(&mut rpass)` keeps working (the binding pattern `let terrain = self.terrain.as_mut();` then using `if let Some(terrain) = ...` twice — first for prepare with the mutable handle, then `self.terrain.as_ref()` is no longer separately needed; restructure minimally so the borrow checker is satisfied: do `prepare` before borrowing gpu mutably for `acquire`, or capture `view_proj`/frustum first. The shape above — terrain `as_mut` next to `gpu` `as_mut` — borrows two disjoint fields and compiles).

- [ ] **Step 6: Run tests + live smoke**

Run: `cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`
Expected: PASS / clean.

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo run --release --manifest-path dabcraft/Cargo.toml &
APP_PID=$!; sleep 10; kill $APP_PID
```
Expected: the M1 island renders **identically** through the indirect path; no validation errors; backface culling visibly correct (fly inside the dirt cube — its near faces vanish, you see through to the far interior faces… which are also culled; the cube is hollow-looking from inside. That is correct backface behavior).

If the island is INVISIBLE: either `INDIRECT_FIRST_INSTANCE` wasn't enabled (silently skipped draws) or winding broke — check the feature first.

- [ ] **Step 7: Commit**

```bash
git add dabcraft/src/render/terrain.rs dabcraft/src/render/gpu.rs dabcraft/src/app.rs dabcraft/assets/shaders/terrain.wgsl
git commit -m "feat: arena-backed terrain renderer with per-section indirect draws"
```

---

### Task 14: Streaming integration — infinite world in the app

Wire everything together: per-frame streaming (drain job results → unload → request gen → request mesh → budgeted uploads), camera tuned for 384-block render distance, extended F3 HUD (spec §8 counters), and removal of the M1 scaffolding (naive mesher, test section). After this task the game IS M2: free flight over an infinite, treed, caved world.

**Files:**
- Modify: `dabcraft/src/game/camera.rs`
- Rewrite (orchestration parts): `dabcraft/src/app.rs`
- Delete: `dabcraft/src/mesh/naive.rs` (and its `mod` declaration)

**Streaming constants (top of app.rs):**

```rust
/// Sections are drawn within this column radius: 12 × 32 = 384 blocks (spec §1).
const RENDER_RADIUS: i32 = 12;
/// Columns are generated one ring wider: meshing a column needs its full
/// 3×3 neighborhood generated.
const LOAD_RADIUS: i32 = RENDER_RADIUS + 1;
/// Hysteresis: unload only beyond LOAD_RADIUS + 2 so walking along a column
/// border doesn't thrash gen/unload.
const UNLOAD_RADIUS: i32 = LOAD_RADIUS + 2;
/// In-flight job caps: keep the rayon queue short so newly-near work isn't
/// stuck behind a distant backlog (priority = submission order, spec §3).
const MAX_GEN_IN_FLIGHT: usize = 12;
const MAX_MESH_IN_FLIGHT: usize = 24;
/// GPU upload budget per frame (sections), to avoid frame spikes (spec §3).
const MAX_UPLOADS_PER_FRAME: usize = 24;
const SEED: i32 = 1337;
```

- [ ] **Step 1: Camera — far plane + sprint (failing tests first)**

In `dabcraft/src/game/camera.rs` tests:

```rust
    #[test]
    fn far_plane_covers_render_distance_diagonal() {
        // 384 blocks horizontally + world height; the corner diagonal is
        // ~sqrt(384² + 384² + 256²) ≈ 601. Far must exceed it.
        assert!(FAR_PLANE >= 700.0);
    }

    #[test]
    fn sprint_multiplies_speed() {
        let mut cam = Camera::new(glam::Vec3::ZERO);
        let mut input = InputState::default();
        input.set_key(KeyCode::KeyW, true);
        cam.fly(&input, 1.0);
        let normal = cam.position.length();
        let mut cam2 = Camera::new(glam::Vec3::ZERO);
        input.set_key(KeyCode::ControlLeft, true);
        cam2.fly(&input, 1.0);
        assert!((cam2.position.length() - normal * SPRINT_MULTIPLIER).abs() < 1e-3);
    }
```

(Adapt imports to the existing test module style in camera.rs.)

Implementation: add `pub const FAR_PLANE: f32 = 800.0;` and `pub const SPRINT_MULTIPLIER: f32 = 8.0;`. Use `FAR_PLANE` in `view_proj`'s perspective call (replace the current hardcoded far). In `fly()`, compute `let speed = FLY_SPEED * if input.is_down(KeyCode::ControlLeft) { SPRINT_MULTIPLIER } else { 1.0 };` and use it for the whole displacement (horizontal + vertical).

Run camera tests: fail → implement → pass.

- [ ] **Step 2: App fields and init**

In `dabcraft/src/app.rs`, add fields to `App`:

```rust
    world: crate::world::chunks::ChunkMap,
    gen: crate::world::gen::WorldGen,
    jobs: crate::world::jobs::Jobs,
    upload_queue: std::collections::VecDeque<(crate::world::chunks::SectionPos, Vec<crate::mesh::quad::PackedQuad>)>,
    stats: FrameStats,
```

with

```rust
#[derive(Default)]
struct FrameStats {
    columns_ready: usize,
    visible_sections: u32,
    resident_sections: u32,
    drawn_quads: u32,
}
```

Initialize in `App::new()`: `world: ChunkMap::default(), gen: WorldGen::new(SEED), jobs: Jobs::new(), upload_queue: VecDeque::new(), stats: FrameStats::default()`.

Camera spawn for terrain (heights reach ~200): `Camera::new(glam::Vec3::new(16.0, 140.0, 16.0))`.

In `resumed()`, delete the test-section upload (`mesh_naive(&build_test_section())` block and `self.quad_count = ...`); the world streams in instead. Delete `build_test_section()` entirely. Delete `dabcraft/src/mesh/naive.rs` and remove `pub mod naive;` from `mesh/mod.rs`. Remove the now-unused `quad_count` field (HUD reads `stats.drawn_quads`).

- [ ] **Step 3: The per-frame world update**

Add to `impl App` (called from `render()` after input/fly, before camera write):

```rust
    /// One streaming step (spec §3, §4): drain finished jobs, unload far
    /// columns, request generation/meshing nearest-first under in-flight
    /// caps, upload finished meshes under a per-frame budget.
    fn update_world(&mut self) {
        use crate::world::chunks::{columns_in_radius, ColumnPos, SectionPos};
        use crate::world::jobs::JobResult;
        let Some(gpu) = self.gpu.as_ref() else { return };

        let center = ColumnPos {
            x: (self.camera.position.x as i32).div_euclid(32),
            z: (self.camera.position.z as i32).div_euclid(32),
        };

        // 1. Drain finished jobs.
        for result in self.jobs.drain() {
            match result {
                JobResult::Generated { pos, data, writes } => {
                    let d2 = (pos.x - center.x).pow(2) + (pos.z - center.z).pow(2);
                    if d2 > UNLOAD_RADIUS * UNLOAD_RADIUS {
                        // Player moved on; drop the result but keep its writes.
                        self.world.queue_writes(writes);
                        continue;
                    }
                    self.world.insert_generated(pos, data, writes);
                }
                JobResult::Meshed { pos, quads } => {
                    if self.world.ready(pos.column()).is_some() {
                        self.upload_queue.push_back((pos, quads));
                    }
                }
            }
        }

        // 2. Unload far columns and free their GPU meshes.
        if let Some(terrain) = self.terrain.as_mut() {
            for pos in self.world.unload_outside(center, UNLOAD_RADIUS) {
                for y in 0..8 {
                    terrain.remove_section(SectionPos { x: pos.x, y, z: pos.z });
                }
            }
        }

        // 3. Request generation, nearest first.
        if self.jobs.gen_in_flight < MAX_GEN_IN_FLIGHT {
            for col in columns_in_radius(center, LOAD_RADIUS) {
                if self.jobs.gen_in_flight >= MAX_GEN_IN_FLIGHT {
                    break;
                }
                if !self.world.contains(col) {
                    self.world.mark_generating(col);
                    self.jobs.spawn_gen(self.gen, col);
                }
            }
        }

        // 4. Request meshing for dirty sections whose 3×3 columns are ready.
        if self.jobs.mesh_in_flight < MAX_MESH_IN_FLIGHT {
            'cols: for col in columns_in_radius(center, RENDER_RADIUS) {
                if self.world.ready(col).is_none() || !self.world.neighbors_ready(col) {
                    continue;
                }
                let dirty: Vec<usize> = self
                    .world
                    .ready(col)
                    .map(|c| (0..8).filter(|&y| c.dirty[y]).collect())
                    .unwrap_or_default();
                for sy in dirty {
                    if self.jobs.mesh_in_flight >= MAX_MESH_IN_FLIGHT {
                        break 'cols;
                    }
                    let pos = SectionPos { x: col.x, y: sy as i32, z: col.z };
                    let hood = self.build_neighborhood(pos);
                    if let Some(c) = self.world.ready_mut(col) {
                        c.dirty[sy] = false;
                    }
                    self.jobs.spawn_mesh(pos, hood);
                }
            }
        }

        // 5. Budgeted GPU uploads.
        if let Some(terrain) = self.terrain.as_mut() {
            for _ in 0..MAX_UPLOADS_PER_FRAME {
                let Some((pos, quads)) = self.upload_queue.pop_front() else { break };
                if self.world.ready(pos.column()).is_none() {
                    continue; // unloaded while queued
                }
                terrain.upload_section(&gpu.queue, pos, &quads);
            }
        }

        self.stats.columns_ready = self.world.ready_count();
    }

    /// Capture the 3×3×3 Arc<Section> neighborhood around a section.
    fn build_neighborhood(&self, pos: crate::world::chunks::SectionPos) -> crate::mesh::neighborhood::MeshNeighborhood {
        use crate::mesh::neighborhood::MeshNeighborhood;
        use crate::world::chunks::ColumnPos;
        let mut hood = MeshNeighborhood::empty();
        for dy in -1..=1 {
            let sy = pos.y + dy;
            if !(0..8).contains(&sy) {
                continue; // above/below the world: stays None → air
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let col = ColumnPos { x: pos.x + dx, z: pos.z + dz };
                    if let Some(c) = self.world.ready(col) {
                        hood.sections[MeshNeighborhood::index(dx, dy, dz)] =
                            Some(c.sections[sy as usize].clone());
                    }
                }
            }
        }
        hood
    }
```

Borrow note: `update_world` takes `&mut self` and internally borrows `self.gpu` immutably (only `gpu.queue` is needed) while mutating `self.terrain`, `self.world`, `self.jobs` — all disjoint fields, but going through `self.` inside closures can fight the checker. The structure above (sequential blocks, no closures over self) compiles; if a step needs restructuring, split borrows locally as M1 did in `render()` — never clone world data to satisfy the checker.

`build_neighborhood` runs before clearing dirty: the order in step 4 (build hood → clear dirty → spawn) ensures a write landing between build and spawn re-dirties and re-meshes later — benign. The `ready` double-lookup keeps borrows simple.

- [ ] **Step 4: Call site + HUD**

In `render()`, after `self.camera.fly(&self.input, dt)`:

```rust
        self.update_world();
```

In the prepare block (from Task 13), capture the stats:

```rust
        if let Some(terrain) = terrain {
            let aspect = gpu.config.width as f32 / gpu.config.height as f32;
            let view_proj = self.camera.view_proj(aspect);
            terrain.write_camera(&gpu.queue, view_proj);
            let frustum = crate::render::frustum::Frustum::from_view_proj(view_proj);
            let stats = terrain.prepare(&gpu.queue, &frustum);
            self.stats.visible_sections = stats.visible_sections;
            self.stats.resident_sections = stats.resident_sections;
            self.stats.drawn_quads = stats.drawn_quads;
        }
```

HUD window contents (replace the three M1 labels; spec §8 counters):

```rust
        let fps = self.fps_smoothed;
        let gpu_ms = self.timer.as_ref().map(|t| t.last_ms).unwrap_or(0.0);
        let cam = self.camera.position;
        let cols = self.stats.columns_ready;
        let resident = self.stats.resident_sections;
        let visible = self.stats.visible_sections;
        let quads = self.stats.drawn_quads;
        let gen_q = self.jobs.gen_in_flight;
        let mesh_q = self.jobs.mesh_in_flight;
        let uploads = self.upload_queue.len();
        let (arena_used, arena_cap) = self
            .terrain
            .as_ref()
            .map(|t| t.arena_usage())
            .unwrap_or((0, 1));
```

```rust
        ui.label(format!("FPS:      {fps:.1}"));
        ui.label(format!("GPU ms:   {gpu_ms:.2}"));
        ui.label(format!("Pos:      {:.0} {:.0} {:.0}", cam.x, cam.y, cam.z));
        ui.label(format!("Columns:  {cols}"));
        ui.label(format!("Sections: {visible}/{resident} drawn/resident"));
        ui.label(format!("Quads:    {quads}"));
        ui.label(format!("Jobs:     gen {gen_q}  mesh {mesh_q}  upload {uploads}"));
        ui.label(format!(
            "Arena:    {:.1}/{:.0} MiB",
            arena_used as f32 * 8.0 / (1 << 20) as f32,
            arena_cap as f32 * 8.0 / (1 << 20) as f32
        ));
```

(Capture the values before the egui closure like M1 does — the closure can't borrow `self`.)

- [ ] **Step 5: Full test + clippy run**

Run: `cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings`
Expected: PASS / clean. (naive.rs tests are gone with the file.)

- [ ] **Step 6: Live validation (the M2 acceptance check)**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo run --release --manifest-path dabcraft/Cargo.toml &
APP_PID=$!; sleep 45; kill $APP_PID
```

While it runs, verify by eye + HUD:
1. World streams in around the spawn within a few seconds; no panics, no wgpu validation errors over the full run.
2. HUD: FPS at/near 120 once the initial generation wave settles; GPU ms well under 8.3; Columns ≈ 550 when fully loaded; Sections drawn < resident (frustum culling working — turn the camera, watch `drawn` change).
3. Terrain has biomes (green plains/forest with trees, sand deserts, snowy peaks, blue oceans), caves visible in cliffsides, trees with canopies crossing chunk borders without seams.
4. Sprint-fly (Ctrl+W) in one direction: new terrain generates ahead, old unloads behind (Columns count stays ~stable; arena MiB stays bounded).
5. AO: block corners and cave mouths show soft darkening; no inverted (brighter-in-corner) shading.
6. Section borders: no missing faces, no double faces, no holes at column seams (the apron meshing at work).

Record the observed numbers (FPS, GPU ms, quads, columns) in the task log for the final review.

- [ ] **Step 7: Commit**

```bash
git add -A dabcraft/src docs
git commit -m "feat: infinite world streaming with budgeted jobs and M2 debug HUD"
```

---

## Execution Notes

- Tasks 1→14 are strictly ordered; no parallel execution (single crate, overlapping files).
- Task-by-task: implement → `cargo test` → `cargo clippy -- -D warnings` → commit. A task is not done with failing checks.
- The greedy mesher (Tasks 5–6) is the highest-risk code: the face/plane mapping table and the AO corner table in this plan were derived by hand from the shipped WGSL FACE tables — if a test contradicts the table, suspect the implementation first, then escalate; do NOT silently "fix" a table to make tests pass.
- Performance sanity (not a gate, just data for the final review): meshing a typical surface section should land in the 50–200 µs band (spec §5); worldgen per column is noise-bound (~2–10 ms) and runs off-thread.
- M3 preview (do not build now): block edits will reuse `dirty_sections_touching` + re-mesh; the persistence thread (M6) will reuse the same job/channel pattern.

## Self-Review Outcomes

- Spec coverage: §4 chunk storage/palette (T2), padded apron (T4), worldgen heightmap+biomes (T7), caves (T8), decoration+pending writes (T9), lifecycle ring (T10/T14); §5 binary greedy meshing (T5), AO+flip (T6), vertex pulling arena + indirect (T13); §6 frustum culling (T11), backface culling (T13); §3 threading model (T10/T14); §8 HUD counters (T14). Cave culling (Checchi) is **M4** per spec §10; flood-fill lighting M4; persistence M6 — intentionally absent here.
- Type consistency: `StructureWrite{pos, block, only_air}` defined T7, used T9/T10; `ColumnData.sections: Vec<Section>` (8) everywhere; `SectionPos.origin()` used by T13/T14; `Arena` slot units are quads in both T12 and T13.
- Known compromise: water/leaves render opaque until M5/M6 (stated in header); `instance_count: 1` per section keeps the door open for GPU culling writing `0` in v2.
