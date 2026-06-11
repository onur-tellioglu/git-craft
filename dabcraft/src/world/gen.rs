// gen.rs is complete but its consumer (the job system) is Task 10.
#![cfg_attr(not(test), allow(dead_code))]

use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};

use crate::world::block::{
    BlockId, DIRT, GRASS, SAND, SNOW_GRASS, STONE, WATER,
};
use crate::world::section::Section;

pub const SEA_LEVEL: i32 = 64;
pub const COLUMN_SECTIONS: usize = 8;
pub const WORLD_HEIGHT: i32 = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Biome {
    Plains,
    Forest,
    Desert,
    Mountains,
    SnowyMountains,
    Ocean,
}

/// One generated 32×256×32 column: 8 stacked sections, bottom-up.
#[derive(Debug)]
pub struct ColumnData {
    pub sections: Vec<Section>,
}

/// A block a structure wants to place outside (or inside) its own column.
/// `only_air`: don't overwrite terrain or other structures (used by leaves).
#[derive(Clone, Copy, Debug)]
pub struct StructureWrite {
    pub pos: glam::IVec3,
    pub block: BlockId,
    pub only_air: bool,
}

/// Wrapper that makes FastNoiseLite behave as Clone + Send + Sync.
///
/// FastNoiseLite contains only i32, f32, and Copy enums — no heap allocations,
/// no Drop impl. The library simply forgot to derive Clone/Send/Sync. We
/// implement them manually; the clone uses ptr::read for a byte-level copy
/// which is correct for this all-plain-data type.
struct NoiseSampler(FastNoiseLite);

// SAFETY: FastNoiseLite is all plain data (i32, f32, Copy enums). No Drop,
// no heap pointers, no interior mutability. Send and Sync are safe.
unsafe impl Send for NoiseSampler {}
unsafe impl Sync for NoiseSampler {}

impl Clone for NoiseSampler {
    fn clone(&self) -> Self {
        // SAFETY: FastNoiseLite has no Drop impl and no heap pointers.
        // ptr::read produces a valid duplicate of the plain-data struct.
        let copied = unsafe { std::ptr::read(&self.0) };
        NoiseSampler(copied)
    }
}

impl NoiseSampler {
    fn get_noise_2d(&self, x: f32, y: f32) -> f32 {
        self.0.get_noise_2d(x, y)
    }

    fn get_noise_3d(&self, x: f32, y: f32, z: f32) -> f32 {
        self.0.get_noise_3d(x, y, z)
    }
}

/// Deterministic worldgen (spec §4). WorldGen is Clone + Send + Sync; rayon
/// jobs receive a cloned copy (all noise state is plain data, clone is cheap).
#[derive(Clone)]
pub struct WorldGen {
    continental: NoiseSampler,
    erosion: NoiseSampler,
    peaks: NoiseSampler,
    temperature: NoiseSampler,
    humidity: NoiseSampler,
    cave_a: NoiseSampler,
    cave_b: NoiseSampler,
    cheese: NoiseSampler,
    pub seed: i32,
}

fn noise(seed: i32, salt: i32, freq: f32, fractal: Option<FractalType>, octaves: i32) -> NoiseSampler {
    let mut n = FastNoiseLite::with_seed(seed.wrapping_add(salt));
    n.set_noise_type(Some(NoiseType::OpenSimplex2));
    n.set_frequency(Some(freq));
    if let Some(f) = fractal {
        n.set_fractal_type(Some(f));
        n.set_fractal_octaves(Some(octaves));
    }
    NoiseSampler(n)
}

impl WorldGen {
    pub fn new(seed: i32) -> Self {
        Self {
            continental: noise(seed, 1, 0.0011, Some(FractalType::FBm), 4),
            erosion: noise(seed, 2, 0.0019, Some(FractalType::FBm), 3),
            peaks: noise(seed, 3, 0.004, Some(FractalType::Ridged), 4),
            temperature: noise(seed, 4, 0.0007, Some(FractalType::FBm), 2),
            humidity: noise(seed, 5, 0.0009, Some(FractalType::FBm), 2),
            // Single-octave 3D noise: caves cost 2-3 samples per voxel and
            // octaves multiply that — tunnels don't need fractal detail.
            cave_a: noise(seed, 6, 0.012, None, 0),
            cave_b: noise(seed, 7, 0.012, None, 0),
            cheese: noise(seed, 8, 0.004, Some(FractalType::FBm), 2),
            seed,
        }
    }

    /// Terrain height (the y of the top solid block), 4..=230.
    pub fn height(&self, x: i32, z: i32) -> i32 {
        let (xf, zf) = (x as f32, z as f32);
        let c = self.continental.get_noise_2d(xf, zf); // -1..1
        let e = (self.erosion.get_noise_2d(xf, zf) + 1.0) * 0.5; // 0..1
        let p = (self.peaks.get_noise_2d(xf, zf) + 1.0) * 0.5; // 0..1
        let base = 64.0 + c * 36.0;
        // Peaks fade out toward (and below) the coast and under high erosion.
        let inland = ((c + 0.1) * 2.5).clamp(0.0, 1.0);
        let mountains = p * p * 90.0 * (1.0 - e * 0.8) * inland;
        (base + mountains).clamp(4.0, 230.0) as i32
    }

    pub fn biome(&self, x: i32, z: i32) -> Biome {
        self.biome_for(self.height(x, z), x, z)
    }

    fn biome_for(&self, height: i32, x: i32, z: i32) -> Biome {
        let t = self.temperature.get_noise_2d(x as f32, z as f32);
        let hu = self.humidity.get_noise_2d(x as f32, z as f32);
        if height < SEA_LEVEL - 1 {
            Biome::Ocean
        } else if height > 108 {
            if t < 0.0 { Biome::SnowyMountains } else { Biome::Mountains }
        } else if t > 0.35 && hu < 0.0 {
            Biome::Desert
        } else if hu > 0.05 {
            Biome::Forest
        } else {
            Biome::Plains
        }
    }

    fn surface_block(biome: Biome, height: i32) -> BlockId {
        match biome {
            Biome::Ocean => SAND,
            Biome::Desert => SAND,
            Biome::SnowyMountains => SNOW_GRASS,
            Biome::Mountains if height > 130 => STONE,
            // Beaches: any biome right at the waterline gets sand.
            _ if height <= SEA_LEVEL + 1 => SAND,
            _ => GRASS,
        }
    }

    fn subsurface_block(biome: Biome) -> BlockId {
        match biome {
            Biome::Desert | Biome::Ocean => SAND,
            _ => DIRT,
        }
    }

    /// Generate column (cx, cz). Returns the column plus structure writes
    /// that fell OUTSIDE it (trees crossing the border — Task 9; empty now).
    pub fn generate_column(&self, cx: i32, cz: i32) -> (ColumnData, Vec<StructureWrite>) {
        let mut sections: Vec<Section> = (0..COLUMN_SECTIONS).map(|_| Section::empty()).collect();
        for lx in 0..32usize {
            for lz in 0..32usize {
                let (wx, wz) = (cx * 32 + lx as i32, cz * 32 + lz as i32);
                let h = self.height(wx, wz);
                let biome = self.biome_for(h, wx, wz);
                for y in 0..=h {
                    let block = if y == h {
                        Self::surface_block(biome, h)
                    } else if y >= h - 3 {
                        Self::subsurface_block(biome)
                    } else if self.is_cave(wx, y, wz, h) {
                        continue; // carved: leave AIR
                    } else {
                        STONE
                    };
                    sections[(y / 32) as usize].set(lx, (y % 32) as usize, lz, block);
                }
                // Flood below sea level with water (only above the terrain surface).
                let water_start = (h + 1).max(0);
                for y in water_start..=SEA_LEVEL {
                    sections[(y / 32) as usize].set(lx, (y % 32) as usize, lz, WATER);
                }
            }
        }
        let writes = Vec::new(); // Task 9 fills this via decoration
        for s in &mut sections {
            s.compact();
        }
        (ColumnData { sections }, writes)
    }

    /// Spaghetti tunnels: carve where two independent 3D noises are both
    /// near zero (their zero-surfaces intersect along winding 1D curves —
    /// inflated to tunnel radius by the threshold). Cheese rooms: rare
    /// low-frequency blobs. Attenuation: never within 8 blocks of the
    /// surface or below y=6, so the surface skin and world floor stay
    /// intact (spec §4 "attenuated near the surface").
    fn is_cave(&self, x: i32, y: i32, z: i32, surface: i32) -> bool {
        if y < 6 || y > surface - 8 {
            return false;
        }
        let (xf, zf) = (x as f32, z as f32);
        let yf = y as f32 * 1.7; // vertical squash → mostly-horizontal tunnels
        let a = self.cave_a.get_noise_3d(xf, yf, zf);
        let b = self.cave_b.get_noise_3d(xf, yf, zf);
        if a * a + b * b < 0.009 {
            return true;
        }
        self.cheese.get_noise_3d(xf, y as f32 * 2.0, zf) > 0.72
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{AIR, STONE, WATER};

    #[test]
    fn same_seed_is_bit_identical() {
        let a = WorldGen::new(42).generate_column(3, -7);
        let b = WorldGen::new(42).generate_column(3, -7);
        assert_eq!(a.0.sections, b.0.sections);
        assert_eq!(a.1.len(), b.1.len());
    }

    #[test]
    fn different_seeds_differ() {
        let a = WorldGen::new(1).generate_column(0, 0);
        let b = WorldGen::new(2).generate_column(0, 0);
        assert_ne!(a.0.sections, b.0.sections, "two seeds producing identical terrain is wrong");
    }

    #[test]
    fn height_is_within_world_bounds() {
        let wg = WorldGen::new(1337);
        for &(x, z) in &[(0, 0), (1000, -2000), (-50000, 99999), (7, 13)] {
            let h = wg.height(x, z);
            assert!((4..=230).contains(&h), "height {h} at ({x},{z})");
        }
    }

    #[test]
    fn column_has_stone_below_surface_and_air_above() {
        let wg = WorldGen::new(1337);
        let (col, _) = wg.generate_column(0, 0);
        let h = wg.height(5, 5);
        let block_at = |y: i32| col.sections[(y / 32) as usize].get(5, (y % 32) as usize, 5);
        assert_eq!(block_at(1), STONE, "deep underground is stone");
        assert_ne!(block_at(h), AIR, "surface block exists at the heightmap value");
        if h >= SEA_LEVEL {
            assert_eq!(block_at(h + 1), AIR, "air above land surface");
        }
        assert_eq!(block_at(250), AIR, "top of world is air");
    }

    #[test]
    fn below_sea_level_terrain_is_flooded() {
        // Find an ocean column within ±64 columns and check water at sea level.
        let wg = WorldGen::new(1337);
        let mut found = false;
        'outer: for cx in -64..64_i32 {
            for cz in -64..64_i32 {
                let (wx, wz) = (cx * 32 + 16, cz * 32 + 16);
                let h = wg.height(wx, wz);
                if h < SEA_LEVEL - 2 {
                    let (col, _) = wg.generate_column(cx, cz);
                    let sea = SEA_LEVEL as usize;
                    let b = col.sections[sea / 32].get(16, sea % 32, 16);
                    assert_eq!(b, WATER, "cell above ocean floor at sea level must be water");
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "no ocean found in 128×128 columns — heightmap tuning is broken");
    }

    #[test]
    fn caves_exist_underground() {
        // Scan a deep slab of several columns for carved air. Bounds wide
        // enough that being outside them means the thresholds are broken.
        let wgen = WorldGen::new(1337);
        let mut carved = 0u32;
        let mut total = 0u32;
        for cx in 0..8 {
            let (col, _) = wgen.generate_column(cx, 0);
            for x in 0..32 {
                for z in 0..32 {
                    let h = wgen.height(cx * 32 + x as i32, z as i32);
                    for y in 8..(h - 8).max(8) {
                        total += 1;
                        if col.sections[(y / 32) as usize].get(x, (y % 32) as usize, z) == AIR {
                            carved += 1;
                        }
                    }
                }
            }
        }
        assert!(total > 0);
        let pct = carved as f32 / total as f32;
        assert!(pct > 0.005, "deep stone is {:.3}% carved — caves missing", pct * 100.0);
        assert!(pct < 0.35, "deep stone is {:.1}% carved — world is swiss cheese", pct * 100.0);
    }

    #[test]
    fn no_carving_near_surface_or_bedrock() {
        let wgen = WorldGen::new(1337);
        for cx in 0..4 {
            let (col, _) = wgen.generate_column(cx, 3);
            for x in 0..32 {
                for z in 0..32 {
                    let h = wgen.height(cx * 32 + x as i32, 3 * 32 + z as i32);
                    for y in 0..6.min(h) {
                        assert_ne!(
                            col.sections[0].get(x, y as usize, z),
                            AIR,
                            "carved below y=6 at ({x},{y},{z})"
                        );
                    }
                    // Subsurface fill covers h-3..=h and carving stops 8 below
                    // the surface, so the surface skin is always intact:
                    for y in (h - 7).max(0)..=h {
                        assert_ne!(
                            col.sections[(y / 32) as usize].get(x, (y % 32) as usize, z),
                            AIR,
                            "surface breach at ({x},{y},{z}), h={h}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn biome_matches_height_extremes() {
        let wg = WorldGen::new(1337);
        for cx in -32..32_i32 {
            for cz in -32..32_i32 {
                let (wx, wz) = (cx * 32, cz * 32);
                let h = wg.height(wx, wz);
                let biome = wg.biome(wx, wz);
                if h < SEA_LEVEL - 1 {
                    assert_eq!(biome, Biome::Ocean);
                }
                if h > 108 {
                    assert!(matches!(biome, Biome::Mountains | Biome::SnowyMountains));
                }
            }
        }
    }
}
