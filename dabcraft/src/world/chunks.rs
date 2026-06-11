// ChunkMap, ColumnPos, SectionPos, and helpers.
// SectionPos is consumed by terrain.rs (Task 13); ChunkMap/ColumnPos/helpers
// are consumed by Task 14 (streaming).
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashMap;
use std::sync::Arc;

use crate::world::r#gen::{apply_write, apply_write_to_section, ColumnData, StructureWrite, COLUMN_SECTIONS};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{OAK_LEAVES, STONE};
    use crate::world::r#gen::StructureWrite;

    fn empty_column_data() -> crate::world::r#gen::ColumnData {
        crate::world::r#gen::ColumnData {
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
        // A write at a section's x=0 edge sits in the +X apron of the west
        // neighbor: that neighbor's mesh must be rebuilt as well.
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
