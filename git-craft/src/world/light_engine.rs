// Incremental flood-fill light engine (spec §4): addition/removal BFS over
// the loaded ChunkMap, per channel. All functions assume the *block* state
// is already current; they fix the light to match.

use std::collections::VecDeque;

use glam::IVec3;

use crate::world::block::BlockId;
use crate::world::chunks::{ChunkMap, ColumnPos};
use crate::world::r#gen::WORLD_HEIGHT;
use crate::world::light::{LightChannel, MAX_LIGHT, light_new_column};
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
        let Some(cur) = map.light(ch, p) else {
            continue;
        };
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
            let Some(block) = map.block_at(n) else {
                continue;
            }; // unloaded: stop
            if block.blocks_light() {
                continue;
            }
            let Some(cur) = map.light(ch, n) else {
                continue;
            };
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
    let Some(old) = map.light(ch, start) else {
        return;
    };
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
    let Some(block) = map.block_at(pos) else {
        return;
    };
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
            let neighbor = ColumnPos {
                x: pos.x + dx,
                z: pos.z + dz,
            };
            if map.ready(neighbor).is_none() {
                continue;
            }
            // World coords of the two facing block planes.
            for i in 0..32i32 {
                for y in 0..WORLD_HEIGHT {
                    let (a, b) = if dx != 0 {
                        let xa = if dx == 1 { pos.x * 32 + 31 } else { pos.x * 32 };
                        (
                            IVec3::new(xa, y, pos.z * 32 + i),
                            IVec3::new(xa + dx, y, pos.z * 32 + i),
                        )
                    } else {
                        let za = if dz == 1 { pos.z * 32 + 31 } else { pos.z * 32 };
                        (
                            IVec3::new(pos.x * 32 + i, y, za),
                            IVec3::new(pos.x * 32 + i, y, za + dz),
                        )
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
#[allow(dead_code)] // debug/oracle helper; also used by tests
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
        col.dirty = [true; crate::world::r#gen::COLUMN_SECTIONS];
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
                                IVec3::new(
                                    c.x * 32 + x as i32,
                                    (sy * 32 + y) as i32,
                                    c.z * 32 + z as i32,
                                ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{AIR, STONE, TORCH};
    use crate::world::chunks::{ChunkMap, ColumnPos};
    use crate::world::r#gen::ColumnData;
    use crate::world::light::{LightChannel, light_new_column};
    use crate::world::section::Section;
    use glam::IVec3;

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
        assert!(
            sky(&map, center) < 15,
            "under the roof center: no direct sky"
        );
        assert_eq!(
            sky(&map, IVec3::new(16, 26, 16)),
            15,
            "above the roof unchanged"
        );
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
        map.insert_generated(
            ColumnPos { x: 1, z: 0 },
            ColumnData { sections },
            light,
            Vec::new(),
        );
        assert_eq!(
            sky(&map, IVec3::new(33, 21, 16)),
            0,
            "tunnel dark before seam healing"
        );
        seed_column_borders(&mut map, ColumnPos { x: 1, z: 0 });
        assert_eq!(
            sky(&map, IVec3::new(32, 21, 16)),
            14,
            "mouth cell lit from the neighbor's 15"
        );
        assert_eq!(
            sky(&map, IVec3::new(36, 21, 16)),
            10,
            "decays one level per step inward"
        );
        assert_eq!(
            sky(&map, IVec3::new(52, 25, 16)),
            0,
            "stone interior stays dark"
        );
    }

    #[test]
    fn incremental_equals_from_scratch() {
        let mut map = flat_map(1);
        let edits: Vec<(IVec3, crate::world::block::BlockId)> = vec![
            (IVec3::new(16, 20, 16), TORCH),
            (IVec3::new(18, 20, 16), STONE),
            (IVec3::new(16, 21, 16), STONE), // box the torch in a little
            (IVec3::new(31, 20, 16), TORCH), // torch on a column border
            (IVec3::new(32, 20, 16), STONE), // wall just across the border
            (IVec3::new(16, 20, 16), AIR),   // remove the first torch again
            (IVec3::new(10, 19, 10), AIR),   // dig into the floor
            (IVec3::new(10, 18, 10), AIR),
            (IVec3::new(11, 18, 10), AIR), // small L-tunnel
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
                            map.light(LightChannel::Sky, p),
                            oracle.light(LightChannel::Sky, p),
                            "sky mismatch at {p}"
                        );
                        assert_eq!(
                            map.light(LightChannel::Block, p),
                            oracle.light(LightChannel::Block, p),
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
            let sections: Vec<Section> = col.sections.iter().map(|a| (**a).clone()).collect();
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
