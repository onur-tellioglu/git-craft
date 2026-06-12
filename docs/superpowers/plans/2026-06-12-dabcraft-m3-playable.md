---
title: dabcraft M3 — Playable
date: 2026-06-12
domain: world-layer
type: enhancement
priority: high
breaking: false
db-migration: false
rls-affecting: false
slice: 3
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files:
  - dabcraft/src/world/chunks.rs
  - dabcraft/src/world/block.rs
  - dabcraft/src/game/*.rs
  - dabcraft/src/render/outline.rs
  - dabcraft/src/render/game_ui.rs
  - dabcraft/src/render/mod.rs
  - dabcraft/src/app.rs
  - dabcraft/assets/shaders/outline.wgsl
trigger-tasks-touched: []
shared-modules-touched: []
---

# dabcraft M3 — Playable Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The game exists: walking physics with gravity, jumping, and swept AABB collision; DDA block break/place with a wireframe outline on the targeted block; a 9-slot hotbar; a crosshair. Fly mode (M2 behavior) remains, toggled with F or double-space.

**Architecture:** All gameplay logic is pure functions over plain data, unit-tested TDD-first: `physics.rs` (axis-separated swept AABB vs. voxels), `raycast.rs` (Amanatides & Woo DDA), `player.rs` (walk/fly state machine), `hotbar.rs` (selection + paging). The world side gains `ChunkMap::block_at`/`set_block`; an edit calls `Arc::make_mut` on the section, then `dirty_sections_touching` so the existing M2 re-mesh path (versioned mesh jobs) rebuilds the affected sections. Rendering gains one tiny pipeline: a LineList wireframe cube (vertex pulling from a const table, no vertex buffer) drawn after terrain in the main pass. UI (crosshair, hotbar, HUD additions) is egui, drawn every frame; F3 now toggles only the debug window.

**Tech Stack:** Rust (edition 2024), wgpu 29, winit 0.30, glam 0.33, egui 0.34, bytemuck 1.25.

**Spec:** `docs/superpowers/specs/2026-06-11-dabcraft-design.md` §7 (gameplay), §9 (physics test cases), §10 (M3). Lighting stays M4 (mesher bakes skylight 15 — edited blocks stay bright; expected). Water stays visually opaque until M5; it is targetable/breakable but non-solid for collision, with slow-sink/swim-up movement.

**No git remote exists** — skip all push/PR/issue steps. Commit locally on branch `feat/m3-playable`.

**Environment:** every shell needs `export PATH="$HOME/.cargo/bin:$PATH"` before cargo commands. All commands run from the repo root with `--manifest-path dabcraft/Cargo.toml`. macOS has no `timeout`; smoke tests use background-run + kill.

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `dabcraft/src/world/chunks.rs` | modify | `block_at` / `set_block` world-position block access |
| `dabcraft/src/world/block.rs` | modify | `PLACEABLE` list, `display_name()`, `color()` (mirrors WGSL palette) |
| `dabcraft/src/game/input.rs` | modify | key/mouse press edges, mouse buttons, scroll-step accumulation |
| `dabcraft/src/game/physics.rs` | create | `Aabb` + `move_aabb` axis-separated swept voxel collision (pure) |
| `dabcraft/src/game/raycast.rs` | create | Amanatides & Woo DDA, `RayHit` (pure) |
| `dabcraft/src/game/player.rs` | create | `Player`: walk (gravity/jump/water) + fly modes (pure over closures) |
| `dabcraft/src/game/hotbar.rs` | create | 9 slots, 1–9 selection, scroll cycling, shift+scroll paging (pure) |
| `dabcraft/src/game/camera.rs` | modify | movement removed (lives in Player); orientation + view_proj only |
| `dabcraft/src/game/mod.rs` | modify | declare new modules |
| `dabcraft/src/render/outline.rs` | create | wireframe cube pipeline for the targeted block |
| `dabcraft/src/render/game_ui.rs` | create | egui crosshair + hotbar drawing |
| `dabcraft/src/render/mod.rs` | modify | declare new modules |
| `dabcraft/assets/shaders/outline.wgsl` | create | LineList cube edges from `vertex_index`, uniform block pos |
| `dabcraft/src/app.rs` | modify | player/interaction/outline/UI wiring, cursor release on Escape |

## Shared Conventions (read before any task)

- **Player AABB:** width 0.6, height 1.8; `Player.position` is the **feet center** (AABB bottom-center). Eye height 1.62 above feet. Camera position = `player.eye()` every frame.
- **Physics constants** (defined in `player.rs`): gravity 32 b/s², jump speed 9.2 (≈1.3-block jump), terminal fall 78 b/s, walk 4.3 b/s, walk sprint ×1.6 (ControlLeft), fly 20 b/s, fly sprint ×8 (unchanged from M2). Water: horizontal speed ×0.5, sink capped at 3 b/s, hold-Space swim up at 4 b/s.
- **Collision solidity ≠ `BlockId::is_solid()`.** For collision and movement: AIR and WATER are passable, everything else solid, **unloaded columns are solid** (the player floats at the load edge instead of falling through ungenerated terrain), y outside 0..256 is air. For raycast targeting: everything except AIR hits (water is visually opaque until M5).
- **World queries are closures** (`&impl Fn(IVec3) -> bool`) so physics/raycast/player are testable with closure worlds — no ChunkMap in unit tests.
- **Modes:** game starts in **Fly** (preserves M2 spawn-while-loading behavior). F or double-Space (two presses within 0.35 s) toggles. Fly mode keeps M2 semantics exactly: no gravity, **no collision**.
- **Interaction:** 6-block reach from the eye along `camera.forward()`. Left = break (instant, creative), right = place against the hit face; both repeat every 0.25 s while held. Place is rejected when the target cell is occupied (anything but AIR/WATER) or intersects the player AABB.
- **Raycast face normal** is the unit outward normal of the struck face; `IVec3::ZERO` when the ray origin starts inside a solid block (placement is skipped in that case).
- **Escape** now releases the cursor (spec §7) instead of exiting; any mouse click re-grabs (and is swallowed). Quit via the window close button / Cmd+Q.
- After every task: `cargo test --manifest-path dabcraft/Cargo.toml` green, then `cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings` clean, then commit. Never `--no-verify`, no Claude co-author trailers.

---

### Task 1: World block access — `block_at` / `set_block`

**Files:**
- Modify: `dabcraft/src/world/chunks.rs`

- [ ] **Step 1: Create the branch**

```bash
git checkout main && git checkout -b feat/m3-playable
```

- [ ] **Step 2: Write the failing tests**

Append inside the existing `tests` module of `dabcraft/src/world/chunks.rs` (it already has `empty_column_data()`):

```rust
#[test]
fn block_at_reads_world_positions_including_negatives() {
    let mut map = ChunkMap::default();
    map.insert_generated(ColumnPos { x: -1, z: 0 }, empty_column_data(), Vec::new());
    assert!(map.set_block(glam::IVec3::new(-31, 70, 5), STONE));
    assert_eq!(map.block_at(glam::IVec3::new(-31, 70, 5)), Some(STONE));
    assert_eq!(map.block_at(glam::IVec3::new(-32, 70, 5)), Some(crate::world::block::AIR));
    assert_eq!(map.block_at(glam::IVec3::new(50, 70, 5)), None, "unloaded column");
    assert_eq!(map.block_at(glam::IVec3::new(-31, -1, 5)), Some(crate::world::block::AIR));
    assert_eq!(map.block_at(glam::IVec3::new(-31, 256, 5)), Some(crate::world::block::AIR));
}

#[test]
fn set_block_dirties_owner_and_border_neighbors() {
    let mut map = ChunkMap::default();
    map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), Vec::new());
    map.insert_generated(ColumnPos { x: 1, z: 0 }, empty_column_data(), Vec::new());
    map.clear_all_dirty(ColumnPos { x: 0, z: 0 });
    map.clear_all_dirty(ColumnPos { x: 1, z: 0 });
    // x=32 is column (1,0)'s west edge: column (0,0)'s +X apron sees it.
    assert!(map.set_block(glam::IVec3::new(32, 64, 5), STONE));
    let east = map.ready(ColumnPos { x: 1, z: 0 }).unwrap();
    assert!(east.dirty[2], "owner section (y=64 → section 2)");
    assert!(east.dirty[1], "y=64 is section 2's bottom row → section 1 apron");
    let west = map.ready(ColumnPos { x: 0, z: 0 }).unwrap();
    assert!(west.dirty[2], "x=32 sits in the west column's +X apron");
}

#[test]
fn set_block_on_unloaded_or_out_of_range_is_rejected() {
    let mut map = ChunkMap::default();
    map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), Vec::new());
    assert!(!map.set_block(glam::IVec3::new(100, 64, 100), STONE));
    assert!(!map.set_block(glam::IVec3::new(5, -1, 5), STONE));
    assert!(!map.set_block(glam::IVec3::new(5, 256, 5), STONE));
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path dabcraft/Cargo.toml block_at`
Expected: COMPILE ERROR — `block_at`/`set_block` not found.

- [ ] **Step 4: Implement**

In `dabcraft/src/world/chunks.rs`, change the top-level import to bring in block ids:

```rust
use crate::world::block::{BlockId, AIR};
```

Add to `impl ChunkMap` (next to `dirty_sections_touching`):

```rust
/// Block at a world position. `None` when the column is not loaded;
/// callers decide whether unloaded means solid (physics) or miss (raycast).
/// Outside world height it is always air.
pub fn block_at(&self, pos: glam::IVec3) -> Option<BlockId> {
    if !(0..256).contains(&pos.y) {
        return Some(AIR);
    }
    let col = self.ready(block_to_column(pos.x, pos.z))?;
    Some(col.sections[(pos.y / 32) as usize].get(
        pos.x.rem_euclid(32) as usize,
        (pos.y % 32) as usize,
        pos.z.rem_euclid(32) as usize,
    ))
}

/// Player edit: set a block and dirty every section whose 34³ padded
/// volume contains the position (the existing M2 re-mesh path picks the
/// dirty flags up next frame). Returns false when the column is not
/// loaded or the position is outside world height.
pub fn set_block(&mut self, pos: glam::IVec3, block: BlockId) -> bool {
    if !(0..256).contains(&pos.y) {
        return false;
    }
    let Some(col) = self.ready_mut(block_to_column(pos.x, pos.z)) else {
        return false;
    };
    // Arc::make_mut: clone-on-write only if a mesh job still holds the
    // old Arc; the in-flight job's result is dropped by the version guard.
    let section = std::sync::Arc::make_mut(&mut col.sections[(pos.y / 32) as usize]);
    section.set(
        pos.x.rem_euclid(32) as usize,
        (pos.y % 32) as usize,
        pos.z.rem_euclid(32) as usize,
        block,
    );
    self.dirty_sections_touching(pos);
    true
}
```

Note: `Arc` is already imported at the top of the file; use the plain `Arc::make_mut(...)` form if so.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: all tests PASS (including all pre-existing ones).

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
git add dabcraft/src/world/chunks.rs
git commit -m "feat: add block_at/set_block world access with re-mesh dirtying"
```

---

### Task 2: Block metadata — placeable list, names, UI colors

**Files:**
- Modify: `dabcraft/src/world/block.rs`

- [ ] **Step 1: Write the failing tests**

Append inside the `tests` module of `dabcraft/src/world/block.rs`:

```rust
#[test]
fn placeable_covers_every_block_except_air() {
    assert_eq!(PLACEABLE.len(), 11);
    assert!(!PLACEABLE.contains(&AIR));
    for id in 1..=11u16 {
        assert!(PLACEABLE.contains(&BlockId(id)), "block {id} missing");
    }
}

#[test]
fn display_names_are_distinct_and_nonempty() {
    let mut names: Vec<&str> = (0..=11u16).map(|id| BlockId(id).display_name()).collect();
    assert!(names.iter().all(|n| !n.is_empty()));
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 12, "names must be unique");
}

#[test]
fn colors_match_the_shader_palette_spot_checks() {
    // Spot-check against terrain.wgsl PALETTE so UI swatches match terrain.
    assert_eq!(GRASS.color(), [0.35, 0.62, 0.22]);
    assert_eq!(WATER.color(), [0.19, 0.36, 0.68]);
    assert_eq!(CACTUS.color(), [0.27, 0.5, 0.21]);
    assert_eq!(AIR.color(), [1.0, 0.0, 1.0], "air = magenta bug color");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib world::block`
Expected: COMPILE ERROR — `PLACEABLE`, `display_name`, `color` not found.

- [ ] **Step 3: Implement**

Add to `dabcraft/src/world/block.rs` after the block constants:

```rust
/// Every block the player can place (creative: everything but air),
/// in hotbar paging order.
pub const PLACEABLE: [BlockId; 11] = [
    GRASS, DIRT, STONE, SAND, SNOW_GRASS, WATER,
    OAK_LOG, OAK_LEAVES, SPRUCE_LOG, SPRUCE_LEAVES, CACTUS,
];
```

Extend `impl BlockId`:

```rust
pub fn display_name(self) -> &'static str {
    match self.0 {
        0 => "Air",
        1 => "Grass",
        2 => "Dirt",
        3 => "Stone",
        4 => "Sand",
        5 => "Snowy Grass",
        6 => "Water",
        7 => "Oak Log",
        8 => "Oak Leaves",
        9 => "Spruce Log",
        10 => "Spruce Leaves",
        11 => "Cactus",
        _ => "Unknown",
    }
}

/// Linear-space RGB mirroring the PALETTE table in terrain.wgsl, used for
/// UI swatches until M6 ships real textures.
pub fn color(self) -> [f32; 3] {
    match self.0 {
        1 => [0.35, 0.62, 0.22],
        2 => [0.45, 0.32, 0.2],
        3 => [0.52, 0.52, 0.54],
        4 => [0.86, 0.81, 0.58],
        5 => [0.91, 0.93, 0.95],
        6 => [0.19, 0.36, 0.68],
        7 => [0.42, 0.31, 0.19],
        8 => [0.23, 0.43, 0.14],
        9 => [0.32, 0.23, 0.14],
        10 => [0.16, 0.3, 0.19],
        11 => [0.27, 0.5, 0.21],
        _ => [1.0, 0.0, 1.0],
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: PASS.

These items are not referenced outside tests until Tasks 7/9/10, and unused items in a binary crate warn under `-D warnings`. Use the same pattern the M2 codebase already uses (see `PackedQuad::unpack`): annotate `PLACEABLE`, `display_name`, and `color` with `#[cfg_attr(not(test), allow(dead_code))]`, and **remove the attribute in the task that first uses each item** (Task 7 uses `PLACEABLE`; Task 10 uses `display_name`/`color`).

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
git add dabcraft/src/world/block.rs
git commit -m "feat: add placeable block list, display names, and UI palette colors"
```

---

### Task 3: Input upgrades — press edges, mouse buttons, scroll steps

**Files:**
- Modify: `dabcraft/src/game/input.rs`

App-side event wiring happens in Task 9; this task only extends `InputState` (pure, fully testable).

- [ ] **Step 1: Write the failing tests**

`dabcraft/src/game/input.rs` has no tests module yet. Add one at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode as K;

    #[test]
    fn key_pressed_fires_once_per_press_edge() {
        let mut input = InputState::default();
        input.set_key(K::KeyF, true);
        assert!(input.key_pressed(K::KeyF));
        assert!(input.is_down(K::KeyF));
        input.end_frame();
        assert!(!input.key_pressed(K::KeyF), "edge consumed");
        assert!(input.is_down(K::KeyF), "still held");
        input.set_key(K::KeyF, true); // OS key-repeat while held
        assert!(!input.key_pressed(K::KeyF), "repeat is not a new edge");
        input.set_key(K::KeyF, false);
        input.set_key(K::KeyF, true);
        assert!(input.key_pressed(K::KeyF), "release + press = new edge");
    }

    #[test]
    fn mouse_buttons_track_edges_and_held_state() {
        let mut input = InputState::default();
        input.set_mouse_button(MouseButton::Left, true);
        assert!(input.mouse_pressed(MouseButton::Left));
        assert!(input.mouse_down(MouseButton::Left));
        assert!(!input.mouse_pressed(MouseButton::Right));
        input.end_frame();
        assert!(!input.mouse_pressed(MouseButton::Left));
        assert!(input.mouse_down(MouseButton::Left));
        input.set_mouse_button(MouseButton::Left, false);
        assert!(!input.mouse_down(MouseButton::Left));
    }

    #[test]
    fn scroll_accumulates_whole_steps_and_keeps_remainder() {
        let mut input = InputState::default();
        input.accumulate_scroll(0.6);
        assert_eq!(input.take_scroll_steps(), 0);
        input.accumulate_scroll(0.6); // 0.6 remainder + 0.6 = 1.2
        assert_eq!(input.take_scroll_steps(), 1);
        input.accumulate_scroll(-2.5);
        assert_eq!(input.take_scroll_steps(), -2);
    }

    #[test]
    fn clear_drops_everything() {
        let mut input = InputState::default();
        input.set_key(K::KeyW, true);
        input.set_mouse_button(MouseButton::Right, true);
        input.accumulate_mouse(3.0, 4.0);
        input.accumulate_scroll(2.0);
        input.clear();
        assert!(!input.is_down(K::KeyW));
        assert!(!input.key_pressed(K::KeyW));
        assert!(!input.mouse_down(MouseButton::Right));
        assert_eq!(input.take_mouse_delta(), (0.0, 0.0));
        assert_eq!(input.take_scroll_steps(), 0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib game::input`
Expected: COMPILE ERROR — `MouseButton`, `key_pressed`, etc. not found.

- [ ] **Step 3: Implement**

Replace the struct and impl in `dabcraft/src/game/input.rs` with (keep the existing doc comments on retained methods):

```rust
use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// Game-relevant mouse buttons (winit's enum carries more; app.rs maps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
}

#[derive(Default)]
pub struct InputState {
    pressed: HashSet<KeyCode>,
    just_pressed: HashSet<KeyCode>,
    mouse_held: HashSet<MouseButton>,
    mouse_just_pressed: HashSet<MouseButton>,
    mouse_delta: (f64, f64),
    scroll: f32,
}

impl InputState {
    pub fn set_key(&mut self, key: KeyCode, down: bool) {
        if down {
            if self.pressed.insert(key) {
                self.just_pressed.insert(key);
            }
        } else {
            self.pressed.remove(&key);
        }
    }

    pub fn is_down(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    /// True only on the frame the key transitioned up→down (OS key-repeat
    /// does not re-fire). Cleared by `end_frame`.
    pub fn key_pressed(&self, key: KeyCode) -> bool {
        self.just_pressed.contains(&key)
    }

    pub fn set_mouse_button(&mut self, button: MouseButton, down: bool) {
        if down {
            if self.mouse_held.insert(button) {
                self.mouse_just_pressed.insert(button);
            }
        } else {
            self.mouse_held.remove(&button);
        }
    }

    pub fn mouse_down(&self, button: MouseButton) -> bool {
        self.mouse_held.contains(&button)
    }

    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.mouse_just_pressed.contains(&button)
    }

    pub fn accumulate_mouse(&mut self, dx: f64, dy: f64) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    /// Mouse deltas accumulate across device events; reset once consumed each frame.
    pub fn take_mouse_delta(&mut self) -> (f64, f64) {
        std::mem::take(&mut self.mouse_delta)
    }

    pub fn accumulate_scroll(&mut self, delta: f32) {
        self.scroll += delta;
    }

    /// Whole scroll steps accumulated since the last call; the fractional
    /// remainder (trackpad deltas) carries over so slow scrolls still land.
    pub fn take_scroll_steps(&mut self) -> i32 {
        let steps = self.scroll.trunc() as i32;
        self.scroll -= steps as f32;
        steps
    }

    /// Consume this frame's press edges. Call once at the end of each frame.
    pub fn end_frame(&mut self) {
        self.just_pressed.clear();
        self.mouse_just_pressed.clear();
    }

    /// winit does not synthesize key-release events for keys held when focus
    /// is lost; stale state must be dropped on focus transitions.
    pub fn clear(&mut self) {
        self.pressed.clear();
        self.just_pressed.clear();
        self.mouse_held.clear();
        self.mouse_just_pressed.clear();
        self.mouse_delta = (0.0, 0.0);
        self.scroll = 0.0;
    }
}
```

New methods are unused until Task 9: annotate `MouseButton`, `key_pressed`, `set_mouse_button`, `mouse_down`, `mouse_pressed`, `accumulate_scroll`, `take_scroll_steps`, and `end_frame` with `#[cfg_attr(not(test), allow(dead_code))]` and remove the attributes in Task 9.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml`
Expected: PASS.

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
git add dabcraft/src/game/input.rs
git commit -m "feat: add press edges, mouse buttons, and scroll steps to InputState"
```

---

### Task 4: Swept AABB physics

**Files:**
- Create: `dabcraft/src/game/physics.rs`
- Modify: `dabcraft/src/game/mod.rs`

Implements spec §7 (swept axis-separated AABB collision, no corner snagging) and §9's physics test cases (corners, exact-touch, high velocity).

- [ ] **Step 1: Declare the module**

In `dabcraft/src/game/mod.rs` add:

```rust
pub mod physics;
```

- [ ] **Step 2: Write the failing tests**

Create `dabcraft/src/game/physics.rs` containing only the tests module for now (plus `use` lines so it compiles as a test target):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{IVec3, Vec3};

    const W: f32 = 0.6;
    const H: f32 = 1.8;

    fn no_blocks(_: IVec3) -> bool {
        false
    }

    /// Infinite flat floor: every cell at y == `y` is solid.
    fn floor_at(y: i32) -> impl Fn(IVec3) -> bool {
        move |c: IVec3| c.y == y
    }

    /// Infinite wall: every cell at x == `x` is solid.
    fn wall_at(x: i32) -> impl Fn(IVec3) -> bool {
        move |c: IVec3| c.x == x
    }

    #[test]
    fn from_feet_builds_centered_box() {
        let b = Aabb::from_feet(Vec3::new(10.0, 64.0, -3.0), W, H);
        assert_eq!(b.min, Vec3::new(9.7, 64.0, -3.3));
        assert_eq!(b.max, Vec3::new(10.3, 65.8, -2.7));
    }

    #[test]
    fn intersects_cell_checks_unit_voxel_overlap() {
        let b = Aabb::from_feet(Vec3::new(0.5, 64.0, 0.5), W, H);
        assert!(b.intersects_cell(IVec3::new(0, 64, 0)));
        assert!(b.intersects_cell(IVec3::new(0, 65, 0)), "1.8 tall spans two cells");
        assert!(!b.intersects_cell(IVec3::new(0, 66, 0)), "head ends at 65.8");
        assert!(!b.intersects_cell(IVec3::new(2, 64, 0)));
    }

    #[test]
    fn falls_freely_without_blocks() {
        let b = Aabb::from_feet(Vec3::new(0.5, 100.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, -10.0, 0.0), &no_blocks);
        assert!((moved.min.y - 90.0).abs() < 1e-4);
        assert_eq!(hit, [false; 3]);
    }

    #[test]
    fn lands_on_floor_even_at_high_velocity() {
        // Spec §9: high-velocity edge case — a 200-block fall in one step
        // must clamp at the floor, not tunnel through it.
        let b = Aabb::from_feet(Vec3::new(0.5, 100.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, -200.0, 0.0), &floor_at(63));
        assert!((moved.min.y - 64.0).abs() < 1e-3, "rests on top of y=63 cells");
        assert!(hit[1]);
        assert!(!hit[0] && !hit[2]);
    }

    #[test]
    fn slides_along_wall_without_snagging() {
        // Axis separation: x is clamped by the wall, z moves the full
        // distance — the classic no-corner-snag behavior.
        let b = Aabb::from_feet(Vec3::new(4.5, 0.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(1.0, 0.0, 2.0), &wall_at(5));
        assert!((moved.max.x - 5.0).abs() < 1e-3, "stopped at the wall plane");
        assert!(hit[0]);
        assert!((moved.min.z - 2.2).abs() < 1e-4, "z slid the full 2.0");
        assert!(!hit[2]);
    }

    #[test]
    fn exact_touch_is_not_a_collision() {
        // Spec §9: exact-touch edge case. Box face exactly on the wall
        // plane, moving parallel to it: full move, no collision flag.
        let b = Aabb::from_feet(Vec3::new(4.7, 0.0, 0.5), W, H); // max.x == 5.0
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, 0.0, 1.0), &wall_at(5));
        assert_eq!(hit, [false; 3]);
        assert!((moved.min.z - 1.2).abs() < 1e-4);
    }

    #[test]
    fn pushing_into_a_touching_wall_moves_zero_not_negative() {
        let start = 5.0 - 0.3 - 1e-4; // resting against the wall with skin
        let b = Aabb::from_feet(Vec3::new(start, 0.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.5, 0.0, 0.0), &wall_at(5));
        assert!(hit[0]);
        assert!((moved.min.x - b.min.x).abs() < 1e-3, "no backward ejection");
    }

    #[test]
    fn ceiling_stops_upward_motion() {
        let b = Aabb::from_feet(Vec3::new(0.5, 64.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, 5.0, 0.0), &floor_at(67));
        assert!((moved.max.y - 67.0).abs() < 1e-3, "head clamped under y=67 cells");
        assert!(hit[1]);
    }

    #[test]
    fn corner_block_does_not_snag_parallel_motion() {
        // Spec §9: corner edge case. A single block beside the path must
        // not stop motion that merely grazes it.
        let solid = |c: IVec3| c == IVec3::new(1, 0, 2);
        let b = Aabb::from_feet(Vec3::new(0.5, 0.0, 0.5), W, H);
        let (moved, hit) = move_aabb(b, Vec3::new(0.0, 0.0, 3.0), &solid);
        // Box spans x 0.2..0.8 — clear of cell x=1; it passes the corner.
        assert_eq!(hit, [false; 3]);
        assert!((moved.min.z - 3.2).abs() < 1e-4);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib game::physics`
Expected: COMPILE ERROR — `Aabb`, `move_aabb` not found.

- [ ] **Step 4: Implement**

Add above the tests module in `dabcraft/src/game/physics.rs`:

```rust
use glam::{IVec3, Vec3};

/// Collision skin: resolved positions stay this far off voxel faces so
/// float equality at boundaries never re-collides on the next frame.
const SKIN: f32 = 1e-4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// Box from a feet-center position (player convention: position is the
    /// AABB bottom-center).
    pub fn from_feet(feet: Vec3, width: f32, height: f32) -> Self {
        let half = width * 0.5;
        Self {
            min: Vec3::new(feet.x - half, feet.y, feet.z - half),
            max: Vec3::new(feet.x + half, feet.y + height, feet.z + half),
        }
    }

    pub fn translated(self, t: Vec3) -> Self {
        Self { min: self.min + t, max: self.max + t }
    }

    /// Does this box overlap the unit voxel at `cell`? (Strict inequality:
    /// exact face contact is not overlap.)
    pub fn intersects_cell(&self, cell: IVec3) -> bool {
        let lo = cell.as_vec3();
        let hi = lo + Vec3::ONE;
        self.min.x < hi.x
            && self.max.x > lo.x
            && self.min.y < hi.y
            && self.max.y > lo.y
            && self.min.z < hi.z
            && self.max.z > lo.z
    }
}

fn make_cell(axis: usize, p: i32, u: usize, i: i32, v: usize, j: i32) -> IVec3 {
    let mut c = [0i32; 3];
    c[axis] = p;
    c[u] = i;
    c[v] = j;
    IVec3::from_array(c)
}

/// Clamp a single-axis move against solid voxels: scan voxel planes from
/// the leading face outward to the move's end. Returns the allowed signed
/// distance (|allowed| ≤ |delta|). Plane scanning (not stepping) means
/// arbitrarily high velocity cannot tunnel.
fn sweep_axis(b: Aabb, axis: usize, delta: f32, is_solid: &impl Fn(IVec3) -> bool) -> f32 {
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    // Cross-section footprint, shrunk by SKIN so faces in exact contact on
    // the perpendicular axes don't count as overlap.
    let u0 = (b.min[u] + SKIN).floor() as i32;
    let u1 = (b.max[u] - SKIN).floor() as i32;
    let v0 = (b.min[v] + SKIN).floor() as i32;
    let v1 = (b.max[v] - SKIN).floor() as i32;
    let blocked = |p: i32| {
        (u0..=u1).any(|i| (v0..=v1).any(|j| is_solid(make_cell(axis, p, u, i, v, j))))
    };

    if delta > 0.0 {
        let lead = b.max[axis];
        let first = (lead + SKIN).floor() as i32;
        let last = (lead + delta).floor() as i32;
        for p in first..=last {
            if blocked(p) {
                return (p as f32 - lead - SKIN).clamp(0.0, delta);
            }
        }
        delta
    } else {
        let lead = b.min[axis];
        let first = (lead - SKIN).floor() as i32;
        let last = (lead + delta).floor() as i32;
        for p in (last..=first).rev() {
            if blocked(p) {
                return ((p + 1) as f32 - lead + SKIN).clamp(delta, 0.0);
            }
        }
        delta
    }
}

/// Move `aabb` by `delta` with axis-separated swept collision, Y first
/// (grounding must resolve before horizontal sliding), then X, then Z.
/// Returns the moved box and per-axis hit flags `[x, y, z]`.
pub fn move_aabb(
    aabb: Aabb,
    delta: Vec3,
    is_solid: &impl Fn(IVec3) -> bool,
) -> (Aabb, [bool; 3]) {
    let mut b = aabb;
    let mut hit = [false; 3];
    for axis in [1usize, 0, 2] {
        let d = delta[axis];
        if d == 0.0 {
            continue;
        }
        let allowed = sweep_axis(b, axis, d, is_solid);
        hit[axis] = allowed != d;
        let mut t = [0.0f32; 3];
        t[axis] = allowed;
        b = b.translated(Vec3::from_array(t));
    }
    (b, hit)
}
```

Note: the scan in the positive branch starts at the cell containing the leading face itself. By invariant the box never overlaps a solid cell, so that cell is air and the extra check is harmless — and it makes a box pressed against a wall (face at `plane − SKIN`) return exactly 0 instead of going negative.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib game::physics`
Expected: 9 tests PASS.

- [ ] **Step 6: Full suite, clippy, commit**

```bash
cargo test --manifest-path dabcraft/Cargo.toml
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
```

`Aabb`/`move_aabb` are unused outside tests until Task 6: annotate the pub items with `#[cfg_attr(not(test), allow(dead_code))]` (remove in Task 6).

```bash
git add dabcraft/src/game/physics.rs dabcraft/src/game/mod.rs
git commit -m "feat: add axis-separated swept AABB voxel collision"
```

---

### Task 5: DDA voxel raycast

**Files:**
- Create: `dabcraft/src/game/raycast.rs`
- Modify: `dabcraft/src/game/mod.rs`

- [ ] **Step 1: Declare the module**

In `dabcraft/src/game/mod.rs` add:

```rust
pub mod raycast;
```

- [ ] **Step 2: Write the failing tests**

Create `dabcraft/src/game/raycast.rs` with the tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{IVec3, Vec3};

    fn only(cell: IVec3) -> impl Fn(IVec3) -> bool {
        move |c: IVec3| c == cell
    }

    #[test]
    fn hits_block_along_positive_x() {
        let hit = raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::X, 6.0, &only(IVec3::new(3, 0, 0)))
            .expect("must hit");
        assert_eq!(hit.block, IVec3::new(3, 0, 0));
        assert_eq!(hit.normal, IVec3::new(-1, 0, 0), "entered through the -X face");
        assert!((hit.distance - 2.5).abs() < 1e-5);
    }

    #[test]
    fn hits_block_in_negative_coordinates() {
        let hit = raycast(
            Vec3::new(-0.5, 0.5, -0.5),
            Vec3::NEG_X,
            6.0,
            &only(IVec3::new(-4, 0, -1)),
        )
        .expect("must hit");
        assert_eq!(hit.block, IVec3::new(-4, 0, -1));
        assert_eq!(hit.normal, IVec3::new(1, 0, 0));
        assert!((hit.distance - 2.5).abs() < 1e-5, "from x=-0.5 to the x=-3 face");
    }

    #[test]
    fn vertical_ray_hits_underside() {
        let hit = raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::Y, 6.0, &only(IVec3::new(0, 5, 0)))
            .expect("must hit");
        assert_eq!(hit.normal, IVec3::new(0, -1, 0));
        assert!((hit.distance - 4.5).abs() < 1e-5);
    }

    #[test]
    fn respects_max_distance() {
        assert!(raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::X, 2.0, &only(IVec3::new(3, 0, 0))).is_none());
    }

    #[test]
    fn misses_when_nothing_is_solid() {
        assert!(raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::ONE, 6.0, &|_| false).is_none());
    }

    #[test]
    fn origin_inside_solid_reports_zero_distance_and_no_face() {
        let hit = raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::X, 6.0, &|_| true).expect("must hit");
        assert_eq!(hit.block, IVec3::new(0, 0, 0));
        assert_eq!(hit.normal, IVec3::ZERO);
        assert_eq!(hit.distance, 0.0);
    }

    #[test]
    fn diagonal_ray_walks_cells_in_order() {
        // From (0.5,0.5,0.5) along normalize(1,1,0): boundary ties resolve
        // x-first (the <= in the axis pick), so the visit order is
        // (0,0,0) (1,0,0) (1,1,0) (2,1,0) (2,2,0)…
        let hit = raycast(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(1.0, 1.0, 0.0).normalize(),
            10.0,
            &only(IVec3::new(2, 2, 0)),
        )
        .expect("must hit");
        assert_eq!(hit.block, IVec3::new(2, 2, 0));
        assert_eq!(hit.normal, IVec3::new(0, -1, 0), "y step entered it last");
    }

    #[test]
    fn zero_direction_returns_none() {
        assert!(raycast(Vec3::new(0.5, 0.5, 0.5), Vec3::ZERO, 6.0, &|_| true).is_none());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib game::raycast`
Expected: COMPILE ERROR — `raycast`, `RayHit` not found.

- [ ] **Step 4: Implement**

Add above the tests module:

```rust
use glam::{IVec3, Vec3};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub block: IVec3,
    /// Unit outward normal of the struck face; `IVec3::ZERO` when the ray
    /// origin starts inside a solid block (no face was crossed).
    pub normal: IVec3,
    /// World-units along the ray to the face crossing.
    pub distance: f32,
}

/// Amanatides & Woo voxel DDA: visits every cell the ray passes through,
/// in order, until `max_dist` (entering a cell exactly at `max_dist` still
/// counts). `dir` need not be normalized; zero direction returns None.
pub fn raycast(
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
    is_solid: &impl Fn(IVec3) -> bool,
) -> Option<RayHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let mut cell = [
        origin.x.floor() as i32,
        origin.y.floor() as i32,
        origin.z.floor() as i32,
    ];
    if is_solid(IVec3::from_array(cell)) {
        return Some(RayHit { block: IVec3::from_array(cell), normal: IVec3::ZERO, distance: 0.0 });
    }
    let o = [origin.x, origin.y, origin.z];
    let d = [dir.x, dir.y, dir.z];
    let mut step = [0i32; 3];
    let mut t_max = [f32::INFINITY; 3]; // ray length to the next boundary, per axis
    let mut t_delta = [f32::INFINITY; 3]; // ray length per whole cell, per axis
    for a in 0..3 {
        if d[a] > 0.0 {
            step[a] = 1;
            t_delta[a] = 1.0 / d[a];
            t_max[a] = ((cell[a] + 1) as f32 - o[a]) / d[a];
        } else if d[a] < 0.0 {
            step[a] = -1;
            t_delta[a] = -1.0 / d[a];
            t_max[a] = (o[a] - cell[a] as f32) / -d[a];
        }
    }
    loop {
        let axis = if t_max[0] <= t_max[1] && t_max[0] <= t_max[2] {
            0
        } else if t_max[1] <= t_max[2] {
            1
        } else {
            2
        };
        let distance = t_max[axis];
        if distance > max_dist {
            return None;
        }
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        if is_solid(IVec3::from_array(cell)) {
            let mut n = [0i32; 3];
            n[axis] = -step[axis];
            return Some(RayHit {
                block: IVec3::from_array(cell),
                normal: IVec3::from_array(n),
                distance,
            });
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib game::raycast`
Expected: 8 tests PASS.

- [ ] **Step 6: Full suite, clippy, commit**

`raycast`/`RayHit` are unused until Task 9: `#[cfg_attr(not(test), allow(dead_code))]` on both (remove in Task 9).

```bash
cargo test --manifest-path dabcraft/Cargo.toml
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
git add dabcraft/src/game/raycast.rs dabcraft/src/game/mod.rs
git commit -m "feat: add Amanatides-Woo DDA voxel raycast"
```

---

### Task 6: Player controller (walk + fly) and camera slim-down

**Files:**
- Create: `dabcraft/src/game/player.rs`
- Modify: `dabcraft/src/game/mod.rs`
- Modify: `dabcraft/src/game/camera.rs`

The camera stops moving itself: movement (including M2's fly) moves into `Player`; `Camera` keeps orientation and projection only. `App` still compiles after this task because `Camera::fly` removal is paired with the app change here (one-line swap is NOT possible yet — the full wiring lands in Task 9, so this task keeps `App` building with a minimal temporary hookup, see Step 7).

- [ ] **Step 1: Declare the module**

In `dabcraft/src/game/mod.rs` add:

```rust
pub mod player;
```

- [ ] **Step 2: Write the failing tests**

Create `dabcraft/src/game/player.rs` with the tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{IVec3, Vec3};
    use winit::keyboard::KeyCode as K;

    const DT: f32 = 0.05;

    fn no_blocks(_: IVec3) -> bool {
        false
    }

    fn floor_at(y: i32) -> impl Fn(IVec3) -> bool {
        move |c: IVec3| c.y == y
    }

    fn keys(down: &[K]) -> crate::game::input::InputState {
        let mut input = crate::game::input::InputState::default();
        for &k in down {
            input.set_key(k, true);
        }
        input
    }

    fn walking_at(pos: Vec3) -> Player {
        let mut p = Player::new(pos);
        p.mode = MoveMode::Walk;
        p
    }

    #[test]
    fn spawns_in_fly_mode() {
        assert_eq!(Player::new(Vec3::ZERO).mode, MoveMode::Fly);
    }

    #[test]
    fn eye_sits_at_eye_height_above_feet() {
        let p = Player::new(Vec3::new(1.0, 64.0, 2.0));
        assert_eq!(p.eye(), Vec3::new(1.0, 64.0 + EYE_HEIGHT, 2.0));
    }

    #[test]
    fn walk_falls_under_gravity_and_lands() {
        let mut p = walking_at(Vec3::new(0.5, 66.0, 0.5));
        let input = keys(&[]);
        for _ in 0..60 {
            p.update(&input, 0.0, DT, &floor_at(63), &|_| false);
        }
        assert!(p.on_ground);
        assert!((p.position.y - 64.0).abs() < 1e-2, "rests on top of y=63");
        assert_eq!(p.velocity.y, 0.0);
    }

    #[test]
    fn jump_fires_only_when_grounded() {
        let mut p = walking_at(Vec3::new(0.5, 64.0 + 1e-4, 0.5));
        let input = keys(&[K::Space]);
        p.update(&input, 0.0, DT, &floor_at(63), &|_| false); // grounds first
        p.update(&input, 0.0, DT, &floor_at(63), &|_| false); // then jumps
        let mut peak = p.position.y;
        for _ in 0..40 {
            p.update(&keys(&[]), 0.0, DT, &floor_at(63), &|_| false);
            peak = peak.max(p.position.y);
        }
        assert!(peak > 65.0, "jump cleared one block, peak {peak}");
        assert!(peak < 65.6, "jump stays near 1.3 blocks, peak {peak}");
    }

    #[test]
    fn airborne_space_does_not_jump() {
        let mut p = walking_at(Vec3::new(0.5, 80.0, 0.5));
        let v0 = p.velocity.y;
        p.update(&keys(&[K::Space]), 0.0, DT, &no_blocks, &|_| false);
        assert!(p.velocity.y < v0, "still accelerating downward");
    }

    #[test]
    fn walks_forward_at_walk_speed() {
        let mut p = walking_at(Vec3::new(0.5, 64.0 + 1e-4, 0.5));
        p.on_ground = true;
        p.update(&keys(&[K::KeyW]), 0.0, DT, &floor_at(63), &|_| false);
        // yaw 0 looks down -Z.
        assert!((p.position.z - (0.5 - WALK_SPEED * DT)).abs() < 1e-4);
        assert_eq!(p.position.x, 0.5);
    }

    #[test]
    fn sprint_scales_walk_speed() {
        let mut p = walking_at(Vec3::new(0.5, 64.0 + 1e-4, 0.5));
        p.update(&keys(&[K::KeyW, K::ControlLeft]), 0.0, DT, &floor_at(63), &|_| false);
        assert!((p.position.z - (0.5 - WALK_SPEED * SPRINT_MULTIPLIER * DT)).abs() < 1e-4);
    }

    #[test]
    fn fly_ignores_gravity_and_blocks() {
        let mut p = Player::new(Vec3::new(0.5, 64.0, 0.5));
        p.update(&keys(&[]), 0.0, DT, &|_| true, &|_| false);
        assert_eq!(p.position, Vec3::new(0.5, 64.0, 0.5), "no input, no motion");
        p.update(&keys(&[K::KeyW]), 0.0, 1.0, &|_| true, &|_| false);
        assert!((p.position.z - (0.5 - FLY_SPEED)).abs() < 1e-4, "moves through solids");
    }

    #[test]
    fn toggle_swaps_mode_and_zeroes_velocity() {
        let mut p = walking_at(Vec3::ZERO);
        p.velocity = Vec3::new(1.0, -5.0, 0.0);
        p.toggle_mode();
        assert_eq!(p.mode, MoveMode::Fly);
        assert_eq!(p.velocity, Vec3::ZERO);
        p.toggle_mode();
        assert_eq!(p.mode, MoveMode::Walk);
    }

    #[test]
    fn water_sinks_slowly_and_swims_up() {
        let everywhere_water = |_: IVec3| true;
        let mut p = walking_at(Vec3::new(0.5, 70.0, 0.5));
        for _ in 0..40 {
            p.update(&keys(&[]), 0.0, DT, &no_blocks, &everywhere_water);
        }
        assert!(p.velocity.y >= -WATER_SINK_SPEED - 1e-4, "sink speed is capped");
        let y_before = p.position.y;
        for _ in 0..10 {
            p.update(&keys(&[K::Space]), 0.0, DT, &no_blocks, &everywhere_water);
        }
        assert!(p.position.y > y_before, "holding space swims upward");
    }

    #[test]
    fn water_halves_walk_speed() {
        let everywhere_water = |_: IVec3| true;
        let mut p = walking_at(Vec3::new(0.5, 70.0, 0.5));
        p.update(&keys(&[K::KeyW]), 0.0, DT, &no_blocks, &everywhere_water);
        let dz = (0.5 - p.position.z) / DT;
        assert!((dz - WALK_SPEED * WATER_SPEED_FACTOR).abs() < 1e-3);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib game::player`
Expected: COMPILE ERROR.

- [ ] **Step 4: Implement the player**

Add above the tests module in `dabcraft/src/game/player.rs`:

```rust
use glam::{IVec3, Vec3};
use winit::keyboard::KeyCode as K;

use crate::game::input::InputState;
use crate::game::physics::{move_aabb, Aabb};

pub const WIDTH: f32 = 0.6;
pub const HEIGHT: f32 = 1.8;
pub const EYE_HEIGHT: f32 = 1.62;
pub const WALK_SPEED: f32 = 4.3; // blocks per second
pub const SPRINT_MULTIPLIER: f32 = 1.6;
pub const FLY_SPEED: f32 = 20.0; // M2's free-flight values, unchanged
pub const FLY_SPRINT_MULTIPLIER: f32 = 8.0;
pub const WATER_SPEED_FACTOR: f32 = 0.5;
pub const WATER_SINK_SPEED: f32 = 3.0;
const GRAVITY: f32 = 32.0;
const JUMP_SPEED: f32 = 9.2; // peak ≈ 9.2²/(2·32) ≈ 1.32 blocks
const TERMINAL_FALL: f32 = 78.0;
const SWIM_UP_SPEED: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveMode {
    Walk,
    Fly,
}

pub struct Player {
    /// Feet center (AABB bottom-center).
    pub position: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub mode: MoveMode,
}

impl Player {
    /// Starts in Fly: the world streams in around the spawn point exactly
    /// like M2; the player toggles to Walk when there is ground to walk on.
    pub fn new(position: Vec3) -> Self {
        Self { position, velocity: Vec3::ZERO, on_ground: false, mode: MoveMode::Fly }
    }

    pub fn aabb(&self) -> Aabb {
        Aabb::from_feet(self.position, WIDTH, HEIGHT)
    }

    pub fn eye(&self) -> Vec3 {
        self.position + Vec3::new(0.0, EYE_HEIGHT, 0.0)
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            MoveMode::Walk => MoveMode::Fly,
            MoveMode::Fly => MoveMode::Walk,
        };
        self.velocity = Vec3::ZERO;
        self.on_ground = false;
    }

    /// One physics step. `is_solid` is the collision query (water and air
    /// are passable; unloaded terrain is the caller's choice — App passes
    /// solid). `is_water` drives swim movement.
    pub fn update(
        &mut self,
        input: &InputState,
        yaw: f32,
        dt: f32,
        is_solid: &impl Fn(IVec3) -> bool,
        is_water: &impl Fn(IVec3) -> bool,
    ) {
        match self.mode {
            MoveMode::Fly => self.update_fly(input, yaw, dt),
            MoveMode::Walk => self.update_walk(input, yaw, dt, is_solid, is_water),
        }
    }

    /// WASD direction on the ground plane from yaw (unnormalized sum).
    fn wish_dir(input: &InputState, yaw: f32) -> Vec3 {
        let forward = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        let right = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let mut dir = Vec3::ZERO;
        if input.is_down(K::KeyW) {
            dir += forward;
        }
        if input.is_down(K::KeyS) {
            dir -= forward;
        }
        if input.is_down(K::KeyD) {
            dir += right;
        }
        if input.is_down(K::KeyA) {
            dir -= right;
        }
        dir
    }

    /// M2 free flight, verbatim semantics: no gravity, no collision.
    fn update_fly(&mut self, input: &InputState, yaw: f32, dt: f32) {
        let mut dir = Self::wish_dir(input, yaw);
        if input.is_down(K::Space) {
            dir += Vec3::Y;
        }
        if input.is_down(K::ShiftLeft) {
            dir -= Vec3::Y;
        }
        if dir != Vec3::ZERO {
            let speed = FLY_SPEED
                * if input.is_down(K::ControlLeft) { FLY_SPRINT_MULTIPLIER } else { 1.0 };
            self.position += dir.normalize() * speed * dt;
        }
    }

    fn update_walk(
        &mut self,
        input: &InputState,
        yaw: f32,
        dt: f32,
        is_solid: &impl Fn(IVec3) -> bool,
        is_water: &impl Fn(IVec3) -> bool,
    ) {
        let feet_cell = self.position.floor().as_ivec3();
        let in_water = is_water(feet_cell) || is_water(self.eye().floor().as_ivec3());

        let wish = Self::wish_dir(input, yaw).normalize_or_zero();
        let mut speed = WALK_SPEED
            * if input.is_down(K::ControlLeft) { SPRINT_MULTIPLIER } else { 1.0 };
        if in_water {
            speed *= WATER_SPEED_FACTOR;
        }
        self.velocity.x = wish.x * speed;
        self.velocity.z = wish.z * speed;

        if in_water {
            // Spec §7: slow sinking, hold-jump to swim upward.
            self.velocity.y = if input.is_down(K::Space) {
                SWIM_UP_SPEED
            } else {
                (self.velocity.y - GRAVITY * 0.25 * dt).max(-WATER_SINK_SPEED)
            };
        } else {
            if self.on_ground && input.is_down(K::Space) {
                self.velocity.y = JUMP_SPEED;
            }
            self.velocity.y = (self.velocity.y - GRAVITY * dt).max(-TERMINAL_FALL);
        }

        let (moved, hit) = move_aabb(self.aabb(), self.velocity * dt, is_solid);
        self.position = Vec3::new(
            (moved.min.x + moved.max.x) * 0.5,
            moved.min.y,
            (moved.min.z + moved.max.z) * 0.5,
        );
        self.on_ground = hit[1] && self.velocity.y <= 0.0;
        if hit[0] {
            self.velocity.x = 0.0;
        }
        if hit[1] {
            self.velocity.y = 0.0;
        }
        if hit[2] {
            self.velocity.z = 0.0;
        }
    }
}
```

Remove the `#[cfg_attr(not(test), allow(dead_code))]` attributes from `Aabb`/`move_aabb` in `physics.rs` (now used). `Player` itself is unused until Task 9 — annotate `Player`, `MoveMode`, and the consts that only Task 9/10 use with the same cfg_attr pattern (remove in Task 9).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib game::player`
Expected: 12 tests PASS.

- [ ] **Step 6: Slim down the camera**

In `dabcraft/src/game/camera.rs`:
1. Delete `Camera::fly` and the consts `FLY_SPEED`, `SPRINT_MULTIPLIER` (keep `FAR_PLANE`).
2. Delete the tests `fly_moves_horizontally_along_yaw_even_when_pitched`, `space_and_shift_move_vertically`, `opposing_keys_cancel`, and `sprint_multiplies_speed` (their behavior now lives in player tests).

- [ ] **Step 7: Temporary app hookup (keeps the build green)**

In `dabcraft/src/app.rs`, `App::render` currently calls `self.camera.fly(&self.input, dt);`. Replace that one line with:

```rust
// Temporary M3 hookup (full wiring in app integration task): fly the
// player and pin the camera to its eye.
{
    let world = &self.world;
    let is_solid = |c: glam::IVec3| match world.block_at(c) {
        Some(b) => b != crate::world::block::AIR && b != crate::world::block::WATER,
        None => true,
    };
    let is_water = |c: glam::IVec3| world.block_at(c) == Some(crate::world::block::WATER);
    self.player.update(&self.input, self.camera.yaw, dt, &is_solid, &is_water);
}
self.camera.position = self.player.eye();
```

Add the field `player: crate::game::player::Player` to `App` and initialize it in `App::new` with `crate::game::player::Player::new(glam::Vec3::new(16.0, 140.0, 16.0))` (the camera's old spawn). Remove the now-stale dead_code attributes from `Player`/`MoveMode` (`toggle_mode` and remaining consts may still need theirs until Tasks 9–10).

- [ ] **Step 8: Full suite, clippy, run, commit**

```bash
cargo test --manifest-path dabcraft/Cargo.toml
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
```

Quick smoke (flight must feel identical to M2):

```bash
cargo run --release --manifest-path dabcraft/Cargo.toml &
APP_PID=$!; sleep 20; kill $APP_PID
```

```bash
git add dabcraft/src/game/player.rs dabcraft/src/game/camera.rs dabcraft/src/game/mod.rs dabcraft/src/app.rs
git commit -m "feat: add player controller with walk physics and fly mode"
```

---

### Task 7: Hotbar

**Files:**
- Create: `dabcraft/src/game/hotbar.rs`
- Modify: `dabcraft/src/game/mod.rs`

- [ ] **Step 1: Declare the module**

In `dabcraft/src/game/mod.rs` add:

```rust
pub mod hotbar;
```

- [ ] **Step 2: Write the failing tests**

Create `dabcraft/src/game/hotbar.rs` with the tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{CACTUS, GRASS, SPRUCE_LEAVES, SPRUCE_LOG, WATER};

    #[test]
    fn defaults_to_first_nine_placeable_blocks() {
        let hb = Hotbar::new();
        assert_eq!(hb.slots[0], GRASS);
        assert_eq!(hb.slots[5], WATER);
        assert_eq!(hb.slots[8], SPRUCE_LOG);
        assert_eq!(hb.selected, 0);
        assert_eq!(hb.selected_block(), GRASS);
    }

    #[test]
    fn select_clamps_to_valid_slots() {
        let mut hb = Hotbar::new();
        hb.select(8);
        assert_eq!(hb.selected, 8);
        hb.select(9); // out of range: ignored
        assert_eq!(hb.selected, 8);
    }

    #[test]
    fn scroll_cycles_selection_both_directions() {
        let mut hb = Hotbar::new();
        hb.scroll(1);
        assert_eq!(hb.selected, 1);
        hb.scroll(-2);
        assert_eq!(hb.selected, 8, "wraps backwards");
        hb.scroll(1);
        assert_eq!(hb.selected, 0, "wraps forwards");
    }

    #[test]
    fn page_scroll_swaps_in_the_remaining_blocks_with_wraparound_fill() {
        let mut hb = Hotbar::new();
        hb.page_scroll(1);
        // Page 1 = PLACEABLE[9..] then wraps to the front of the list.
        assert_eq!(hb.slots[0], SPRUCE_LEAVES);
        assert_eq!(hb.slots[1], CACTUS);
        assert_eq!(hb.slots[2], GRASS, "wraparound fill keeps slots populated");
        hb.page_scroll(1);
        assert_eq!(hb.slots[0], GRASS, "two pages total, wraps back to page 0");
        hb.page_scroll(-1);
        assert_eq!(hb.slots[0], SPRUCE_LEAVES, "negative paging wraps too");
    }

    #[test]
    fn selection_survives_paging() {
        let mut hb = Hotbar::new();
        hb.select(4);
        hb.page_scroll(1);
        assert_eq!(hb.selected, 4);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib game::hotbar`
Expected: COMPILE ERROR.

- [ ] **Step 4: Implement**

Add above the tests module:

```rust
use crate::world::block::{BlockId, PLACEABLE};

pub const SLOT_COUNT: usize = 9;

/// Creative hotbar: 9 slots over the PLACEABLE list. Keys 1–9 / wheel pick
/// a slot; shift+wheel pages the whole bar through every placeable block
/// (spec §7). Pages wrap; a short last page wraps around to the list front
/// so no slot is ever empty.
pub struct Hotbar {
    pub slots: [BlockId; SLOT_COUNT],
    pub selected: usize,
    page: usize,
}

impl Hotbar {
    pub fn new() -> Self {
        let mut hb = Self { slots: [PLACEABLE[0]; SLOT_COUNT], selected: 0, page: 0 };
        hb.fill_from_page();
        hb
    }

    fn page_count() -> usize {
        PLACEABLE.len().div_ceil(SLOT_COUNT)
    }

    fn fill_from_page(&mut self) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            *slot = PLACEABLE[(self.page * SLOT_COUNT + i) % PLACEABLE.len()];
        }
    }

    /// Keys 1–9 (0-based). Out-of-range is ignored.
    pub fn select(&mut self, slot: usize) {
        if slot < SLOT_COUNT {
            self.selected = slot;
        }
    }

    /// Mouse wheel: cycle the selected slot (positive = next, wraps).
    pub fn scroll(&mut self, steps: i32) {
        self.selected =
            (self.selected as i32 + steps).rem_euclid(SLOT_COUNT as i32) as usize;
    }

    /// Shift+wheel: page the bar through the full placeable list (wraps).
    pub fn page_scroll(&mut self, steps: i32) {
        let pages = Self::page_count() as i32;
        self.page = (self.page as i32 + steps).rem_euclid(pages) as usize;
        self.fill_from_page();
    }

    pub fn selected_block(&self) -> BlockId {
        self.slots[self.selected]
    }
}

impl Default for Hotbar {
    fn default() -> Self {
        Self::new()
    }
}
```

Remove the dead_code attribute from `PLACEABLE` in `block.rs` (now used). `Hotbar` is unused until Task 9 — same cfg_attr pattern on the struct and methods (remove in Task 9).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib game::hotbar`
Expected: 5 tests PASS.

- [ ] **Step 6: Full suite, clippy, commit**

```bash
cargo test --manifest-path dabcraft/Cargo.toml
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
git add dabcraft/src/game/hotbar.rs dabcraft/src/game/mod.rs dabcraft/src/world/block.rs
git commit -m "feat: add 9-slot creative hotbar with selection and paging"
```

---

### Task 8: Block outline renderer

**Files:**
- Create: `dabcraft/assets/shaders/outline.wgsl`
- Create: `dabcraft/src/render/outline.rs`
- Modify: `dabcraft/src/render/mod.rs`

A dedicated tiny pipeline (spec §7): LineList topology, 12 cube edges = 24 vertices pulled from a const table in the shader (no vertex buffer — same vertex-pulling philosophy as terrain). Drawn inside the main render pass after terrain, depth-tested `LessEqual` with depth writes off, slightly inflated to avoid z-fighting. The shader is loaded at startup and is not hot-reload-watched (only `terrain.wgsl` is; outline is 30 lines and stable).

- [ ] **Step 1: Write the failing shader-validation test**

Create `dabcraft/src/render/outline.rs` with only:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn shipped_outline_shader_is_valid() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/outline.wgsl"
        ))
        .unwrap();
        assert!(crate::render::hot_reload::validate_wgsl(&src).is_ok());
    }
}
```

In `dabcraft/src/render/mod.rs` add:

```rust
pub mod outline;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib render::outline`
Expected: FAIL — `outline.wgsl` does not exist (unwrap panics).

- [ ] **Step 3: Write the shader**

Create `dabcraft/assets/shaders/outline.wgsl`:

```wgsl
struct OutlineUniform {
    view_proj: mat4x4<f32>,
    // xyz = min corner of the targeted block (world space); w unused.
    block: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: OutlineUniform;

// 12 cube edges as 24 corner indices. Corner bit decode: x = bit 0,
// y = bit 1, z = bit 2.
const EDGES = array<u32, 24>(
    0u, 1u, 1u, 5u, 5u, 4u, 4u, 0u,  // bottom ring (y = 0)
    2u, 3u, 3u, 7u, 7u, 6u, 6u, 2u,  // top ring (y = 1)
    0u, 2u, 1u, 3u, 5u, 7u, 4u, 6u,  // verticals
);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let c = EDGES[vi];
    let corner = vec3<f32>(f32(c & 1u), f32((c >> 1u) & 1u), f32((c >> 2u) & 1u));
    // Inflate slightly around the cube center so the lines sit just off the
    // block faces instead of z-fighting them.
    let pos = u.block.xyz + vec3(0.5) + (corner - vec3(0.5)) * 1.004;
    return u.view_proj * vec4(pos, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4(0.05, 0.05, 0.05, 1.0);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path dabcraft/Cargo.toml --lib render::outline`
Expected: PASS.

- [ ] **Step 5: Implement the renderer**

Add above the tests module in `dabcraft/src/render/outline.rs`:

```rust
use glam::IVec3;

use crate::render::depth::DEPTH_FORMAT;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OutlineUniform {
    view_proj: [[f32; 4]; 4],
    block: [f32; 4],
}

/// Wireframe outline for the targeted block: one 24-vertex LineList draw,
/// vertices pulled from a const table in the shader.
pub struct OutlineRenderer {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    target: Option<IVec3>,
}

impl OutlineRenderer {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        shader_source: &str,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("outline"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("outline"),
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
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("outline"),
            size: std::mem::size_of::<OutlineUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("outline"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("outline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("outline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[], // vertex pulling: corner table lives in the shader
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                // Read-only depth: lines sit ON the block faces; LessEqual
                // keeps them visible without disturbing the depth buffer.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
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
        });
        Self { pipeline, buffer, bind_group, target: None }
    }

    /// Update the targeted block (None hides the outline). Call before the
    /// render pass; write_buffer lands before the subsequent submit.
    pub fn set_target(&mut self, queue: &wgpu::Queue, view_proj: glam::Mat4, target: Option<IVec3>) {
        self.target = target;
        if let Some(t) = target {
            let uniform = OutlineUniform {
                view_proj: view_proj.to_cols_array_2d(),
                block: [t.x as f32, t.y as f32, t.z as f32, 0.0],
            };
            queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&uniform));
        }
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass<'_>) {
        if self.target.is_none() {
            return;
        }
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..24, 0..1);
    }
}
```

`OutlineRenderer` is unused until Task 9 — cfg_attr dead_code on the struct and impl items (remove in Task 9).

- [ ] **Step 6: Full suite, clippy, commit**

```bash
cargo test --manifest-path dabcraft/Cargo.toml
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
git add dabcraft/assets/shaders/outline.wgsl dabcraft/src/render/outline.rs dabcraft/src/render/mod.rs
git commit -m "feat: add wireframe block outline pipeline"
```

---

### Task 9: App wiring — events, interaction, outline, cursor release

**Files:**
- Modify: `dabcraft/src/app.rs`

This task is integration (no new pure logic): wire mouse events into `InputState`, drive break/place from the raycast, draw the outline, handle mode toggles and hotbar keys, and switch Escape from "quit" to "release cursor" (spec §7). Remove every remaining `#[cfg_attr(not(test), allow(dead_code))]` added in Tasks 3, 5, 6, 7, 8 for items this task now uses.

- [ ] **Step 1: Add fields and constants**

In `dabcraft/src/app.rs`, near the other consts:

```rust
/// Block interaction reach from the eye (spec §7).
const REACH: f32 = 6.0;
/// Held-button break/place repeat interval (creative).
const EDIT_REPEAT: f32 = 0.25;
/// Two Space presses within this window toggle walk/fly.
const DOUBLE_TAP_WINDOW: f32 = 0.35;
```

Add fields to `App` (player was added in Task 6):

```rust
hotbar: crate::game::hotbar::Hotbar,
outline: Option<crate::render::outline::OutlineRenderer>,
target: Option<crate::game::raycast::RayHit>,
last_space_press: Option<std::time::Instant>,
break_timer: f32,
place_timer: f32,
cursor_grabbed: bool,
```

Initialize in `App::new`: `hotbar: crate::game::hotbar::Hotbar::new()`, `outline: None`, `target: None`, `last_space_press: None`, `break_timer: 0.0`, `place_timer: 0.0`, `cursor_grabbed: false`.

- [ ] **Step 2: Cursor grab helper + resumed() changes**

Add to `impl App`:

```rust
/// Grab (lock + hide) or release the cursor. Input state is cleared on
/// both transitions so half-held keys/buttons don't leak across.
fn set_cursor_grab(&mut self, grab: bool) {
    let Some(window) = &self.window else { return };
    if grab {
        if window.set_cursor_grab(winit::window::CursorGrabMode::Locked).is_err() {
            let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Confined);
        }
        window.set_cursor_visible(false);
    } else {
        let _ = window.set_cursor_grab(winit::window::CursorGrabMode::None);
        window.set_cursor_visible(true);
    }
    self.cursor_grabbed = grab;
    self.input.clear();
}
```

In `resumed()`:
1. Create the outline renderer right after the terrain renderer (the gpu binding is still in scope):

```rust
let outline_src = std::fs::read_to_string(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/shaders/outline.wgsl"
))
.expect("outline.wgsl missing");
self.outline = Some(crate::render::outline::OutlineRenderer::new(
    &gpu.device,
    gpu.config.format,
    &outline_src,
));
```

2. Replace the existing inline cursor-grab block at the end with `self.window = Some(window); self.set_cursor_grab(true);` (note: `set_cursor_grab` needs `self.window`, so set the field first).

- [ ] **Step 3: Window event changes**

In `window_event`:

1. **Escape arm:** replace `event_loop.exit()` with `self.set_cursor_grab(false); return;` (quit stays on CloseRequested / Cmd+Q).

2. **After the egui filter**, extend the final `match event` with mouse arms:

```rust
WindowEvent::MouseInput { state, button, .. } => {
    if !self.cursor_grabbed {
        // Click-to-refocus: re-grab and swallow the click so it doesn't
        // break a block.
        if state.is_pressed() {
            self.set_cursor_grab(true);
        }
        return;
    }
    let mapped = match button {
        winit::event::MouseButton::Left => Some(crate::game::input::MouseButton::Left),
        winit::event::MouseButton::Right => Some(crate::game::input::MouseButton::Right),
        _ => None,
    };
    if let Some(b) = mapped {
        self.input.set_mouse_button(b, state.is_pressed());
    }
}
WindowEvent::MouseWheel { delta, .. } => {
    let steps = match delta {
        winit::event::MouseScrollDelta::LineDelta(_, y) => y,
        winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
    };
    self.input.accumulate_scroll(steps);
}
```

- [ ] **Step 4: Frame-loop input handling in render()**

Replace the mouse-delta line and the Task 6 temporary hookup with:

```rust
let (dx, dy) = self.input.take_mouse_delta();
if self.cursor_grabbed {
    self.camera.apply_mouse_delta(dx, dy);
}

// Mode toggles: F, or double-tapped Space (spec §7).
if self.input.key_pressed(KeyCode::KeyF) {
    self.player.toggle_mode();
}
if self.input.key_pressed(KeyCode::Space) {
    let now = std::time::Instant::now();
    if self
        .last_space_press
        .is_some_and(|t| now.duration_since(t).as_secs_f32() < DOUBLE_TAP_WINDOW)
    {
        self.player.toggle_mode();
        self.last_space_press = None;
    } else {
        self.last_space_press = Some(now);
    }
}

// Hotbar: 1–9 select, wheel cycles, shift+wheel pages (spec §7).
const DIGITS: [KeyCode; 9] = [
    KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3,
    KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6,
    KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9,
];
for (i, key) in DIGITS.iter().enumerate() {
    if self.input.key_pressed(*key) {
        self.hotbar.select(i);
    }
}
let scroll_steps = self.input.take_scroll_steps();
if scroll_steps != 0 {
    if self.input.is_down(KeyCode::ShiftLeft) {
        self.hotbar.page_scroll(scroll_steps);
    } else {
        self.hotbar.scroll(scroll_steps);
    }
}

// Player movement against the loaded world. Unloaded columns are solid:
// the player floats at the load edge instead of falling through terrain
// that hasn't generated yet.
{
    let world = &self.world;
    let is_solid = |c: glam::IVec3| match world.block_at(c) {
        Some(b) => b != crate::world::block::AIR && b != crate::world::block::WATER,
        None => true,
    };
    let is_water = |c: glam::IVec3| world.block_at(c) == Some(crate::world::block::WATER);
    self.player.update(&self.input, self.camera.yaw, dt, &is_solid, &is_water);
}
self.camera.position = self.player.eye();

self.update_interaction(dt);
```

(`use winit::keyboard::KeyCode;` is already imported in app.rs.)

- [ ] **Step 5: Interaction method**

Add to `impl App`:

```rust
/// Raycast the targeted block and apply break/place edits. Edits call
/// ChunkMap::set_block, whose dirty flags feed the existing M2 re-mesh
/// path in update_world (versioned jobs drop any stale in-flight mesh).
fn update_interaction(&mut self, dt: f32) {
    use crate::game::input::MouseButton;
    use crate::world::block::{AIR, WATER};

    self.break_timer = (self.break_timer - dt).max(0.0);
    self.place_timer = (self.place_timer - dt).max(0.0);

    // Target anything non-air: water is visually opaque until M5, so
    // targeting (and breaking) it matches what the player sees.
    self.target = {
        let world = &self.world;
        let hits = |c: glam::IVec3| world.block_at(c).is_some_and(|b| b != AIR);
        crate::game::raycast::raycast(self.camera.position, self.camera.forward(), REACH, &hits)
    };
    if !self.cursor_grabbed {
        return;
    }
    let Some(hit) = self.target else { return };

    // Left: instant break (creative), repeating while held.
    if self.input.mouse_pressed(MouseButton::Left)
        || (self.input.mouse_down(MouseButton::Left) && self.break_timer == 0.0)
    {
        self.world.set_block(hit.block, AIR);
        self.break_timer = EDIT_REPEAT;
    }

    // Right: place against the hit face. No face when the ray started
    // inside a block. Rejected if the cell is occupied (water counts as
    // replaceable) or intersects the player AABB (spec §7).
    if (self.input.mouse_pressed(MouseButton::Right)
        || (self.input.mouse_down(MouseButton::Right) && self.place_timer == 0.0))
        && hit.normal != glam::IVec3::ZERO
    {
        let cell = hit.block + hit.normal;
        let free = self.world.block_at(cell).is_some_and(|b| b == AIR || b == WATER);
        if free && !self.player.aabb().intersects_cell(cell) {
            self.world.set_block(cell, self.hotbar.selected_block());
            self.place_timer = EDIT_REPEAT;
        }
    }
}
```

- [ ] **Step 6: Outline draw + frame restructuring**

In `render()`:

1. The view_proj is currently computed inside the `if let Some(terrain)` prepare block. Hoist it so the outline can use it:

```rust
let aspect = gpu.config.width as f32 / gpu.config.height as f32;
let view_proj = self.camera.view_proj(aspect);
if let Some(terrain) = self.terrain.as_mut() {
    terrain.write_camera(&gpu.queue, view_proj);
    let frustum = crate::render::frustum::Frustum::from_view_proj(view_proj);
    let stats = terrain.prepare(&gpu.queue, &frustum);
    self.stats.visible_sections = stats.visible_sections;
    self.stats.resident_sections = stats.resident_sections;
    self.stats.drawn_quads = stats.drawn_quads;
}
if let Some(outline) = self.outline.as_mut() {
    outline.set_target(&gpu.queue, view_proj, self.target.map(|h| h.block));
}
```

2. Inside the main render pass, after `terrain.draw(&mut rpass);`:

```rust
if let Some(outline) = self.outline.as_ref() {
    outline.draw(&mut rpass);
}
```

3. At the very end of `render()` (after `frame.present()`), consume this frame's press edges:

```rust
self.input.end_frame();
```

- [ ] **Step 7: HUD additions**

In the egui HUD closure, capture before the closure:

```rust
let mode = match self.player.mode {
    crate::game::player::MoveMode::Walk => "walk",
    crate::game::player::MoveMode::Fly => "fly",
};
let target_label = self
    .target
    .map(|h| format!("{} {} {}", h.block.x, h.block.y, h.block.z))
    .unwrap_or_else(|| "—".to_string());
```

and add labels inside the Debug HUD window:

```rust
ui.label(format!("Mode:     {mode}"));
ui.label(format!("Target:   {target_label}"));
```

- [ ] **Step 8: Full suite, clippy, smoke run**

```bash
cargo test --manifest-path dabcraft/Cargo.toml
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
cargo run --release --manifest-path dabcraft/Cargo.toml &
APP_PID=$!; sleep 25; kill $APP_PID
```

Manual checks during the smoke run (controller/user plays for ~25 s):
- F toggles to walk; player falls and lands on terrain; WASD walks, Space jumps, Ctrl sprints.
- Double-space toggles back to fly; flight feels like M2.
- Looking at terrain shows a dark wireframe on the targeted block within 6 blocks; none beyond.
- Left click breaks (hole appears, neighbors re-mesh seamlessly); right click places the selected block; placing into your own feet is rejected.
- Escape frees the cursor; clicking re-grabs without breaking a block.
- HUD shows Mode/Target lines.

- [ ] **Step 9: Commit**

```bash
git add dabcraft/src/app.rs dabcraft/src/game/input.rs dabcraft/src/game/player.rs dabcraft/src/game/hotbar.rs dabcraft/src/game/raycast.rs dabcraft/src/render/outline.rs
git commit -m "feat: wire player interaction, block edits, outline, and cursor release"
```

(The non-app files appear only if dead_code attributes were removed there.)

---

### Task 10: Game UI — crosshair and hotbar (egui)

**Files:**
- Create: `dabcraft/src/render/game_ui.rs`
- Modify: `dabcraft/src/render/mod.rs`
- Modify: `dabcraft/src/app.rs`

Per spec §7 all UI goes through egui. Game UI (crosshair, hotbar) draws **every frame**; F3 now toggles only the debug window. UI drawing has no pure logic to unit-test (colors/names were tested in Task 2); validation is visual.

- [ ] **Step 1: Implement the UI module**

Create `dabcraft/src/render/game_ui.rs`:

```rust
use crate::world::block::BlockId;

/// Center-screen crosshair: two hairline segments on the Foreground layer.
pub fn draw_crosshair(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("crosshair"),
    ));
    let c = ctx.screen_rect().center();
    let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200));
    painter.line_segment([c - egui::vec2(8.0, 0.0), c + egui::vec2(8.0, 0.0)], stroke);
    painter.line_segment([c - egui::vec2(0.0, 8.0), c + egui::vec2(0.0, 8.0)], stroke);
}

/// Bottom-center hotbar: 9 color swatches (block colors mirror the terrain
/// palette until M6 textures), white border on the selected slot, selected
/// block name above.
pub fn draw_hotbar(ctx: &egui::Context, slots: &[BlockId; 9], selected: usize) {
    egui::Area::new(egui::Id::new("hotbar"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(slots[selected].display_name())
                        .color(egui::Color32::WHITE)
                        .strong(),
                );
                ui.horizontal(|ui| {
                    for (i, block) in slots.iter().enumerate() {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
                        let c = block.color();
                        // Rgba is linear-space; conversion to Color32 applies
                        // the sRGB transfer, matching the shader's look.
                        let fill = egui::Color32::from(egui::Rgba::from_rgb(c[0], c[1], c[2]));
                        ui.painter().rect_filled(rect.shrink(2.0), 4.0, fill);
                        let stroke = if i == selected {
                            egui::Stroke::new(2.0, egui::Color32::WHITE)
                        } else {
                            egui::Stroke::new(1.0, egui::Color32::from_gray(90))
                        };
                        ui.painter().rect_stroke(rect.shrink(2.0), 4.0, stroke, egui::StrokeKind::Outside);
                    }
                });
            });
        });
}
```

In `dabcraft/src/render/mod.rs` add:

```rust
pub mod game_ui;
```

Remove the dead_code attributes from `display_name`/`color` in `block.rs` (now used).

- [ ] **Step 2: Restructure the egui draw in app.rs**

In `render()`, egui currently draws only when `hud_visible` (with a `drain_input` else-branch). Game UI must draw every frame, so the branch goes away. Capture before the closure (next to the existing captures):

```rust
let hotbar_slots = self.hotbar.slots;
let hotbar_selected = self.hotbar.selected;
let hud_visible = self.hud_visible;
```

Replace the whole `let egui_cmds = if self.hud_visible { ... } else { ... };` block with:

```rust
let egui_cmds = if let Some(egui) = &mut self.egui {
    let window = self.window.as_ref().unwrap().clone();
    let config = &gpu.config;
    let cmds = egui.draw(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &window,
        &view,
        config,
        |ctx| {
            crate::render::game_ui::draw_crosshair(ctx);
            crate::render::game_ui::draw_hotbar(ctx, &hotbar_slots, hotbar_selected);
            if hud_visible {
                egui::Window::new("Debug HUD")
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        // ... existing labels, plus the Task 9 Mode/Target lines ...
                    });
            }
        },
    );
    Some(cmds)
} else {
    None
};
```

Keep every existing HUD label exactly as it is — only the surrounding structure changes. `EguiLayer::drain_input` loses its last caller: delete the method in `dabcraft/src/render/egui_layer.rs` (egui now draws — and therefore drains — every frame).

- [ ] **Step 3: Full suite, clippy, smoke run**

```bash
cargo test --manifest-path dabcraft/Cargo.toml
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
cargo run --release --manifest-path dabcraft/Cargo.toml &
APP_PID=$!; sleep 20; kill $APP_PID
```

Manual checks:
- Crosshair centered; hotbar bottom-center with 9 colored slots and the selected name above.
- Keys 1–9 and the wheel move the white selection border; shift+wheel swaps the slot contents (page 2 shows Spruce Leaves/Cactus first).
- Placing uses the selected block (verify color of the placed block).
- F3 hides/shows only the debug window; crosshair and hotbar stay.

- [ ] **Step 4: Commit**

```bash
git add dabcraft/src/render/game_ui.rs dabcraft/src/render/mod.rs dabcraft/src/render/egui_layer.rs dabcraft/src/app.rs dabcraft/src/world/block.rs
git commit -m "feat: add crosshair and hotbar UI, decouple game UI from F3"
```

---

### Task 11: Final validation

**Files:** none new — verification + any straggler fixes.

- [ ] **Step 1: Full check battery**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --manifest-path dabcraft/Cargo.toml
cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings
cargo build --release --manifest-path dabcraft/Cargo.toml
```

Expected: all green, no warnings.

- [ ] **Step 2: Playable acceptance run**

```bash
cargo run --release --manifest-path dabcraft/Cargo.toml &
APP_PID=$!; sleep 60; kill $APP_PID 2>/dev/null
```

Acceptance checklist (spec §7 / §10 — "The game exists"):
- [ ] Spawn in fly mode; world streams in (M2 behavior intact, HUD FPS comparable to M2).
- [ ] F / double-space toggles walking; gravity, landing, jumping, sprint all work; no corner snagging walking against block edges; no falling through terrain at high speed.
- [ ] Walk into water (oceans are at sea level y≈64): movement slows, player sinks slowly, holding Space swims up.
- [ ] Break and place blocks up to 6 blocks away; outline tracks the targeted block; placement against every face direction works; cannot place inside yourself.
- [ ] Edits re-mesh seamlessly across section borders (break a block at a 32-boundary and check the neighbor face appears).
- [ ] Hotbar + crosshair render; selection via keys/wheel; paging via shift+wheel.
- [ ] Escape releases the cursor; click re-grabs.
- [ ] F3 toggles the debug window only; per-frame stats (sections, quads, jobs) still live.

- [ ] **Step 3: Tidy and close out**

Any acceptance failure: fix, test, and amend/commit on the task that owns the behavior. When green:

```bash
git log --oneline main..feat/m3-playable
```

Confirm the history is one commit per task (plus fixes). Report completion — merging `feat/m3-playable` into `main` is the controller's call, same as M2.

---

## Self-Review Notes (already applied)

- **Spec coverage §7/§10:** walking physics (T4/T6), double-space/F toggle (T9), water movement (T6), DDA raycast 6-block reach (T5/T9), instant break / place-against-face / player-AABB rejection (T9), wireframe outline pipeline (T8), hotbar 1–9 + wheel + shift+wheel paging (T7/T9/T10), crosshair (T10), egui-only UI (T10), Escape releases cursor (T9). Spec §9 physics edge cases — corners, exact-touch, high velocity — are explicit tests in T4.
- **Type consistency:** `Aabb::from_feet/intersects_cell/translated` (T4) match uses in T6/T9; `RayHit{block,normal,distance}` (T5) matches T9; `Hotbar::{select,scroll,page_scroll,selected_block}` (T7) match T9/T10; `InputState::{key_pressed,mouse_pressed,mouse_down,take_scroll_steps,end_frame}` (T3) match T9.
- **Deliberate scope cuts:** no hold-to-break progress bar (creative = instant), no outline hot-reload (static 30-line shader), fly mode has no collision (preserves M2 free flight; spec doesn't require it), lighting untouched until M4 (edits keep baked skylight 15 — placed blocks look fullbright in caves until then).
