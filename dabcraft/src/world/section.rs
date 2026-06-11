use crate::world::block::{BlockId, AIR};

pub const SECTION_SIZE: usize = 32;

pub struct Section {
    blocks: Box<[BlockId; SECTION_SIZE * SECTION_SIZE * SECTION_SIZE]>,
}

impl Section {
    pub fn empty() -> Self {
        Self { blocks: Box::new([AIR; SECTION_SIZE * SECTION_SIZE * SECTION_SIZE]) }
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < SECTION_SIZE && y < SECTION_SIZE && z < SECTION_SIZE);
        (y * SECTION_SIZE + z) * SECTION_SIZE + x
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockId {
        self.blocks[Self::index(x, y, z)]
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, block: BlockId) {
        self.blocks[Self::index(x, y, z)] = block;
    }

    /// Out-of-bounds counts as air (M1: no neighbor apron yet).
    pub fn get_or_air(&self, x: i32, y: i32, z: i32) -> BlockId {
        let r = 0..SECTION_SIZE as i32;
        if r.contains(&x) && r.contains(&y) && r.contains(&z) {
            self.get(x as usize, y as usize, z as usize)
        } else {
            AIR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{AIR, STONE};

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
}
