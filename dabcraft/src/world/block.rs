#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u16);

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

impl BlockId {
    pub fn is_solid(self) -> bool {
        self != AIR
    }
}

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
