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

/// Every block the player can place (creative: everything but air),
/// in hotbar paging order.
pub const PLACEABLE: [BlockId; 11] = [
    GRASS, DIRT, STONE, SAND, SNOW_GRASS, WATER,
    OAK_LOG, OAK_LEAVES, SPRUCE_LOG, SPRUCE_LEAVES, CACTUS,
];

impl BlockId {
    pub fn is_solid(self) -> bool {
        self != AIR
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
            _ => "Unknown",
        }
    }

    /// Linear-space RGB mirroring the PALETTE table in terrain.wgsl, used for
    /// UI swatches until M6 ships real textures.
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
        let mut names: Vec<&str> =
            (0..=max_block_id()).map(|id| BlockId(id).display_name()).collect();
        assert!(names.iter().all(|n| !n.is_empty() && *n != "Unknown"));
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "names must be unique");
    }

    #[test]
    fn colors_match_the_shader_palette() {
        // Parse the PALETTE table out of terrain.wgsl and compare every
        // entry: the Rust table and the shader table must never drift.
        let wgsl = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/terrain.wgsl"
        ))
        .unwrap();
        let table = wgsl
            .split("const PALETTE")
            .nth(1)
            .expect("PALETTE table present")
            .split("\n);")
            .next()
            .unwrap(); // the array body only, up to its closing paren
        let palette: Vec<[f32; 3]> = table
            .split("vec3(")
            .skip(1) // text before the first vec3
            .map(|chunk| {
                let nums: Vec<f32> = chunk
                    .split(')')
                    .next()
                    .unwrap()
                    .split(',')
                    .map(|n| n.trim().parse().unwrap())
                    .collect();
                [nums[0], nums[1], nums[2]]
            })
            .collect();
        assert_eq!(palette.len() as u16, max_block_id() + 1, "one entry per block id");
        for (id, expected) in palette.iter().enumerate() {
            assert_eq!(
                BlockId(id as u16).color(),
                *expected,
                "color drift for block {id} vs terrain.wgsl"
            );
        }
    }

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
