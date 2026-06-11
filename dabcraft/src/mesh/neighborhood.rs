// MeshNeighborhood: 3×3×3 Arc-captured sections for off-thread meshing. Binary consumer: Tasks 13/14.
#![cfg_attr(not(test), allow(dead_code))]

use std::sync::Arc;

use crate::mesh::padded::PaddedSection;
use crate::world::block::{BlockId, AIR};
use crate::world::section::Section;

/// The 3×3×3 sections around a mesh target, captured as Arc clones so the
/// mesh job reads consistent data with zero copying. Missing neighbors
/// (world top/bottom; unloaded horizontals are prevented by the scheduler)
/// read as air.
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
