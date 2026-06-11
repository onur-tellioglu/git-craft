#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockId(pub u16);

pub const AIR: BlockId = BlockId(0);
pub const GRASS: BlockId = BlockId(1);
pub const DIRT: BlockId = BlockId(2);
pub const STONE: BlockId = BlockId(3);

impl BlockId {
    pub fn is_solid(self) -> bool {
        self != AIR
    }
}
