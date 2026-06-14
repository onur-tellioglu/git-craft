// MeshNeighborhood: 3×3×3 Arc-captured sections for off-thread meshing.

use std::sync::Arc;

use crate::mesh::padded::PaddedSection;
use crate::world::block::{BlockId, AIR};
use crate::world::light::{pack_light, LightData, MAX_LIGHT};
use crate::world::section::Section;

/// The 3×3×3 sections around a mesh target, captured as Arc clones so the
/// mesh job reads consistent data with zero copying. Missing neighbors
/// (world top/bottom; unloaded horizontals are prevented by the scheduler)
/// read as air (blocks) or open sky (light).
pub struct MeshNeighborhood {
    pub sections: [Option<Arc<Section>>; 27],
    pub light: [Option<Arc<LightData>>; 27],
}

impl MeshNeighborhood {
    pub fn empty() -> Self {
        Self {
            sections: std::array::from_fn(|_| None),
            light: std::array::from_fn(|_| None),
        }
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
}
