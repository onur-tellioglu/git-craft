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
pub const TORCH: BlockId = BlockId(12);

/// Every block the player can place (creative: everything but air),
/// in hotbar paging order.
pub const PLACEABLE: [BlockId; 12] = [
    GRASS,
    DIRT,
    STONE,
    SAND,
    SNOW_GRASS,
    WATER,
    OAK_LOG,
    OAK_LEAVES,
    SPRUCE_LOG,
    SPRUCE_LEAVES,
    CACTUS,
    TORCH,
];

impl BlockId {
    /// Opaque cube face for meshing (and, via app closures, collision).
    /// Does NOT imply light-blocking: a torch is a solid cube that light
    /// passes through — see `blocks_light`.
    pub fn is_solid(self) -> bool {
        self != AIR
    }

    /// Does this block stop flood-fill light? Everything except air and
    /// torches. Water blocks light fully in M4 (it also renders opaque);
    /// per-block attenuation can arrive with transparency in M5.
    pub fn blocks_light(self) -> bool {
        self != AIR && self != TORCH
    }

    /// Blocklight level seeded at this block's cell (spec §4: BFS from
    /// emitters). Torches emit 14, like vanilla.
    pub fn light_emission(self) -> u8 {
        if self == TORCH { 14 } else { 0 }
    }

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
            12 => "Torch",
            _ => "Unknown",
        }
    }

    /// Linear-space RGB that seeds the procedural material atlas (per-block base
    /// albedo) and drives the hotbar color swatches. The PALETTE constant in
    /// terrain.wgsl was removed in M6c; this is now the single source of truth
    /// for each block's base color.
    pub fn color(self) -> [f32; 3] {
        match self.0 {
            0 => [1.0, 0.0, 1.0], // air: magenta = bug color, mirrors PALETTE[0]
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
            12 => [0.95, 0.71, 0.30],
            _ => [1.0, 0.0, 1.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Highest registered block id, derived so tests track registry growth.
    fn max_block_id() -> u16 {
        PLACEABLE.iter().map(|b| b.0).max().unwrap()
    }

    #[test]
    fn placeable_covers_every_block_except_air() {
        assert_eq!(PLACEABLE.len(), max_block_id() as usize);
        assert!(!PLACEABLE.contains(&AIR));
        for id in 1..=max_block_id() {
            assert!(PLACEABLE.contains(&BlockId(id)), "block {id} missing");
        }
    }

    #[test]
    fn display_names_are_distinct_and_nonempty() {
        let mut names: Vec<&str> = (0..=max_block_id())
            .map(|id| BlockId(id).display_name())
            .collect();
        assert!(names.iter().all(|n| !n.is_empty() && *n != "Unknown"));
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "names must be unique");
    }

    #[test]
    fn ids_are_stable() {
        // Persisted worlds (M6) depend on these exact values; never renumber.
        let expected: [(BlockId, u16); 13] = [
            (AIR, 0),
            (GRASS, 1),
            (DIRT, 2),
            (STONE, 3),
            (SAND, 4),
            (SNOW_GRASS, 5),
            (WATER, 6),
            (OAK_LOG, 7),
            (OAK_LEAVES, 8),
            (SPRUCE_LOG, 9),
            (SPRUCE_LEAVES, 10),
            (CACTUS, 11),
            (TORCH, 12),
        ];
        for (block, id) in expected {
            assert_eq!(block.0, id);
        }
    }

    #[test]
    fn only_air_is_not_solid() {
        assert!(!AIR.is_solid());
        for id in 1..=12u16 {
            assert!(BlockId(id).is_solid());
        }
    }

    #[test]
    fn torch_is_registered_and_placeable() {
        assert_eq!(TORCH.0, 12, "persisted ids are stable; torch is 12");
        assert!(PLACEABLE.contains(&TORCH));
        assert_eq!(TORCH.display_name(), "Torch");
        assert!(TORCH.is_solid(), "torch renders as an opaque cube in M4");
    }

    #[test]
    fn only_air_and_torch_pass_light() {
        assert!(!AIR.blocks_light());
        assert!(
            !TORCH.blocks_light(),
            "a torch must not shadow the sky shaft it sits in"
        );
        for id in 1..=max_block_id() {
            if BlockId(id) == TORCH {
                continue;
            }
            assert!(BlockId(id).blocks_light(), "block {id} must block light");
        }
    }

    #[test]
    fn torch_is_the_only_emitter() {
        assert_eq!(TORCH.light_emission(), 14);
        for id in 0..=max_block_id() {
            if BlockId(id) == TORCH {
                continue;
            }
            assert_eq!(BlockId(id).light_emission(), 0, "block {id} must not emit");
        }
    }
}
