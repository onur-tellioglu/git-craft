use crate::world::block::{AIR, BlockId};
use crate::world::light::{LightData, MAX_LIGHT, pack_light};
use crate::world::section::{SECTION_SIZE, Section};

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
                    if x == 0
                        || x == PADDED - 1
                        || y == 0
                        || y == PADDED - 1
                        || z == 0
                        || z == PADDED - 1
                    {
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
    fn air_padded_defaults_to_full_sky() {
        let p = PaddedSection::air();
        assert_eq!(p.light_packed(0, 0, 0), 0x0F);
        assert_eq!(p.light_packed(33, 33, 33), 0x0F);
    }

    #[test]
    fn interior_copies_section_at_plus_one_offset() {
        let mut s = Section::empty();
        s.set(0, 0, 0, GRASS);
        s.set(31, 31, 31, STONE);
        let light = LightData::uniform(MAX_LIGHT, 0);
        let p = PaddedSection::build(&s, &light, |_, _, _| (AIR, 0x0F));
        assert_eq!(p.get(1, 1, 1), GRASS);
        assert_eq!(p.get(32, 32, 32), STONE);
        assert_eq!(p.get(2, 1, 1), AIR);
    }

    #[test]
    fn apron_filled_from_neighbor_closure_in_local_coords() {
        let s = Section::empty();
        let light = LightData::uniform(MAX_LIGHT, 0);
        // Neighbor closure sees -1..=32; tag each apron cell by how many
        // coordinates are out of range so faces/edges/corners are separable.
        let p = PaddedSection::build(&s, &light, |x, y, z| {
            let outside = u16::from(!(0..32).contains(&x))
                + u16::from(!(0..32).contains(&y))
                + u16::from(!(0..32).contains(&z));
            (BlockId(outside), 0x0F)
        });
        assert_eq!(p.get(0, 5, 5), BlockId(1), "face apron");
        assert_eq!(p.get(33, 5, 5), BlockId(1));
        assert_eq!(p.get(0, 0, 5), BlockId(2), "edge apron");
        assert_eq!(p.get(33, 33, 33), BlockId(3), "corner apron");
        assert_eq!(
            p.get(5, 5, 5),
            AIR,
            "interior comes from the section, not the closure"
        );
    }

    #[test]
    fn set_overrides_for_test_scenarios() {
        let mut p = PaddedSection::air();
        p.set(17, 3, 9, DIRT);
        assert_eq!(p.get(17, 3, 9), DIRT);
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
        assert_eq!(
            p.light_packed(6, 7, 6),
            0x0B,
            "interior light at +1 padded offset"
        );
        assert_eq!(p.light_packed(6, 6, 6), 0x00, "dark interior cell");
        assert_eq!(p.light_packed(0, 5, 5), 0x23, "apron from the closure");
    }
}
