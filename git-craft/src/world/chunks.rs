// ChunkMap, ColumnPos, SectionPos, and helpers.

use std::collections::HashMap;
use std::sync::Arc;

use crate::world::block::{BlockId, AIR};
use crate::world::light::{LightChannel, LightData, MAX_LIGHT};
use crate::world::r#gen::{apply_write_to_section, ColumnData, StructureWrite, COLUMN_SECTIONS, WORLD_HEIGHT};
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
    pub light: [Arc<LightData>; COLUMN_SECTIONS],
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

    /// Queue writes for columns that may not exist yet (also used when a
    /// generation result is dropped because the player moved away).
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
    /// (applying + dirtying) or to the pending queue.
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
        let section = Arc::make_mut(&mut col.sections[(pos.y / 32) as usize]);
        section.set(
            pos.x.rem_euclid(32) as usize,
            (pos.y % 32) as usize,
            pos.z.rem_euclid(32) as usize,
            block,
        );
        self.dirty_sections_touching(pos);
        true
    }

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
    /// the cell. Returns false when out of world, unloaded, or unchanged.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{OAK_LEAVES, STONE};
    use crate::world::light::{LightChannel, LightData};
    use crate::world::r#gen::StructureWrite;

    fn empty_column_data() -> crate::world::r#gen::ColumnData {
        crate::world::r#gen::ColumnData {
            sections: (0..8).map(|_| crate::world::section::Section::empty()).collect(),
        }
    }

    fn dark_light() -> [crate::world::light::LightData; COLUMN_SECTIONS] {
        std::array::from_fn(|_| crate::world::light::LightData::dark())
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
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        let col = map.ready(ColumnPos { x: 0, z: 0 }).unwrap();
        assert_eq!(col.sections[2].get(5, 6, 5), OAK_LEAVES); // y 70 = section 2, local 6
    }

    #[test]
    fn insert_routes_writes_to_ready_columns_and_marks_dirty() {
        let mut map = ChunkMap::default();
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        map.clear_all_dirty(ColumnPos { x: 0, z: 0 });
        // Column (1,0) generates and spills a write into (0,0):
        map.insert_generated(
            ColumnPos { x: 1, z: 0 },
            empty_column_data(),
            dark_light(),
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
            dark_light(),
            vec![StructureWrite { pos: glam::IVec3::new(40, 70, 5), block: STONE, only_air: false }],
        );
        map.insert_generated(ColumnPos { x: 1, z: 0 }, empty_column_data(), dark_light(), Vec::new());
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
                map.insert_generated(ColumnPos { x: dx, z: dz }, empty_column_data(), dark_light(), Vec::new());
            }
        }
        assert!(!map.neighbors_ready(ColumnPos { x: 0, z: 0 }));
        map.insert_generated(ColumnPos { x: 1, z: 1 }, empty_column_data(), dark_light(), Vec::new());
        assert!(map.neighbors_ready(ColumnPos { x: 0, z: 0 }));
    }

    #[test]
    fn border_write_dirties_adjacent_sections_too() {
        // A write at a section's x=0 edge sits in the +X apron of the west
        // neighbor: that neighbor's mesh must be rebuilt as well.
        let mut map = ChunkMap::default();
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        map.insert_generated(ColumnPos { x: -1, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        map.clear_all_dirty(ColumnPos { x: 0, z: 0 });
        map.clear_all_dirty(ColumnPos { x: -1, z: 0 });
        map.insert_generated(
            ColumnPos { x: 5, z: 5 }, // unrelated column whose gen spilled this write:
            empty_column_data(),
            dark_light(),
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
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        map.insert_generated(ColumnPos { x: 9, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        let removed = map.unload_outside(ColumnPos { x: 0, z: 0 }, 5);
        assert_eq!(removed, vec![ColumnPos { x: 9, z: 0 }]);
        assert!(map.ready(ColumnPos { x: 9, z: 0 }).is_none());
        assert!(map.ready(ColumnPos { x: 0, z: 0 }).is_some());
    }

    #[test]
    fn block_at_reads_world_positions_including_negatives() {
        let mut map = ChunkMap::default();
        map.insert_generated(ColumnPos { x: -1, z: 0 }, empty_column_data(), dark_light(), Vec::new());
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
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        map.insert_generated(ColumnPos { x: 1, z: 0 }, empty_column_data(), dark_light(), Vec::new());
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
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        assert!(!map.set_block(glam::IVec3::new(100, 64, 100), STONE));
        assert!(!map.set_block(glam::IVec3::new(5, -1, 5), STONE));
        assert!(!map.set_block(glam::IVec3::new(5, 256, 5), STONE));
    }

    #[test]
    fn light_accessors_follow_world_conventions() {
        let mut map = ChunkMap::default();
        let mut light = dark_light();
        light[2] = LightData::uniform(15, 0); // sections 2: y 64..96 fully sky-lit
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), light, Vec::new());
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
        map.insert_generated(ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
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
        let touched = map.insert_generated(
            ColumnPos { x: 0, z: 0 }, empty_column_data(), dark_light(), Vec::new());
        assert_eq!(touched, vec![glam::IVec3::new(5, 70, 5)], "pending write applied at insert");
        // Outside-write routed into the now-ready column:
        let touched = map.insert_generated(
            ColumnPos { x: 1, z: 0 },
            empty_column_data(),
            dark_light(),
            vec![StructureWrite { pos: glam::IVec3::new(31, 70, 5), block: STONE, only_air: false }],
        );
        assert_eq!(touched, vec![glam::IVec3::new(31, 70, 5)]);
        // A write queued for an absent column is NOT reported (nothing applied yet).
        let touched = map.insert_generated(
            ColumnPos { x: 5, z: 5 },
            empty_column_data(),
            dark_light(),
            vec![StructureWrite { pos: glam::IVec3::new(200, 70, 200), block: STONE, only_air: false }],
        );
        assert!(touched.is_empty());
    }
}
