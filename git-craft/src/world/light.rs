// Per-section voxel light storage (spec §4): two 4-bit channels per voxel,
// skylight in the low nibble, blocklight in the high nibble.

pub const MAX_LIGHT: u8 = 15;

const SIZE: usize = 32;
const VOLUME: usize = SIZE * SIZE * SIZE;

/// Which flood-fill channel an operation targets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LightChannel {
    Sky,
    Block,
}

pub fn pack_light(sky: u8, block: u8) -> u8 {
    debug_assert!(sky <= MAX_LIGHT && block <= MAX_LIGHT);
    sky | (block << 4)
}

/// 32³ light values. `Uniform` covers the dominant cases (all sky-15 above
/// ground, all dark underground) in one byte; the first divergent write
/// promotes to `Dense` (32 KiB). Same voxel index order as `Section`.
#[derive(Clone, Debug, PartialEq)]
pub enum LightData {
    Uniform(u8),
    Dense(Box<[u8; VOLUME]>),
}

impl LightData {
    #[allow(dead_code)] // used in tests and future mesh work (M5+)
    pub fn dark() -> Self {
        LightData::Uniform(0)
    }

    #[allow(dead_code)] // used in tests
    pub fn uniform(sky: u8, block: u8) -> Self {
        LightData::Uniform(pack_light(sky, block))
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < SIZE && y < SIZE && z < SIZE);
        (y * SIZE + z) * SIZE + x
    }

    pub fn packed(&self, x: usize, y: usize, z: usize) -> u8 {
        match self {
            LightData::Uniform(v) => *v,
            LightData::Dense(d) => d[Self::index(x, y, z)],
        }
    }

    pub fn sky(&self, x: usize, y: usize, z: usize) -> u8 {
        self.packed(x, y, z) & 0x0F
    }

    pub fn block_light(&self, x: usize, y: usize, z: usize) -> u8 {
        self.packed(x, y, z) >> 4
    }

    pub fn get(&self, ch: LightChannel, x: usize, y: usize, z: usize) -> u8 {
        match ch {
            LightChannel::Sky => self.sky(x, y, z),
            LightChannel::Block => self.block_light(x, y, z),
        }
    }

    /// Returns true when the stored value actually changed.
    fn set_packed(&mut self, x: usize, y: usize, z: usize, new: u8) -> bool {
        let i = Self::index(x, y, z);
        match self {
            LightData::Uniform(v) => {
                if *v == new {
                    return false;
                }
                let mut dense = Box::new([*v; VOLUME]);
                dense[i] = new;
                *self = LightData::Dense(dense);
                true
            }
            LightData::Dense(d) => {
                if d[i] == new {
                    return false;
                }
                d[i] = new;
                true
            }
        }
    }

    pub fn set_sky(&mut self, x: usize, y: usize, z: usize, v: u8) -> bool {
        debug_assert!(v <= MAX_LIGHT);
        let old = self.packed(x, y, z);
        self.set_packed(x, y, z, (old & 0xF0) | v)
    }

    pub fn set_block_light(&mut self, x: usize, y: usize, z: usize, v: u8) -> bool {
        debug_assert!(v <= MAX_LIGHT);
        let old = self.packed(x, y, z);
        self.set_packed(x, y, z, (old & 0x0F) | (v << 4))
    }

    pub fn set(&mut self, ch: LightChannel, x: usize, y: usize, z: usize, v: u8) -> bool {
        match ch {
            LightChannel::Sky => self.set_sky(x, y, z, v),
            LightChannel::Block => self.set_block_light(x, y, z, v),
        }
    }

    /// Bulk-decode all 32768 packed bytes (hot path for padded-buffer fill).
    #[allow(dead_code)] // wired into mesh padding in M5
    pub fn unpack_into(&self, out: &mut [u8]) {
        assert_eq!(out.len(), VOLUME);
        match self {
            LightData::Uniform(v) => out.fill(*v),
            LightData::Dense(d) => out.copy_from_slice(&d[..]),
        }
    }

    /// Build from a 32768-entry skylight slice (blocklight 0), collapsing to
    /// Uniform when every value matches. Used by column light generation.
    pub fn from_sky_slice(sky: &[u8]) -> Self {
        assert_eq!(sky.len(), VOLUME);
        let first = sky[0];
        if sky.iter().all(|&v| v == first) {
            return LightData::Uniform(pack_light(first, 0));
        }
        let mut data = Box::new([0u8; VOLUME]);
        for (d, &s) in data.iter_mut().zip(sky) {
            *d = pack_light(s, 0);
        }
        LightData::Dense(data)
    }
}

use std::collections::VecDeque;

use crate::world::block::AIR;
use crate::world::r#gen::COLUMN_SECTIONS;
use crate::world::section::Section;

const WORLD_H: usize = 256;

fn cidx(x: usize, y: usize, z: usize) -> usize {
    (y * SIZE + z) * SIZE + x
}

/// Initial skylight for a freshly generated column (pure; runs in the rayon
/// gen job). Vertical seed: 15 from the top until the first light-blocking
/// block. Then an in-column BFS: sideways/up lose 1 per step, level-15
/// falls downward undimmed (vanilla rule). Blocklight starts 0 — worldgen
/// places no emitters. Cross-column seams are healed by
/// `light_engine::seed_column_borders` when neighbors are loaded.
pub fn light_new_column(sections: &[Section]) -> [LightData; COLUMN_SECTIONS] {
    assert_eq!(sections.len(), COLUMN_SECTIONS);
    let mut blocks = vec![AIR; SIZE * SIZE * WORLD_H];
    for (s, section) in sections.iter().enumerate() {
        section.unpack_into(&mut blocks[s * VOLUME..(s + 1) * VOLUME]);
    }
    let mut sky = vec![0u8; SIZE * SIZE * WORLD_H];
    let mut queue: VecDeque<(usize, usize, usize, u8)> = VecDeque::new();
    for x in 0..SIZE {
        for z in 0..SIZE {
            for y in (0..WORLD_H).rev() {
                if blocks[cidx(x, y, z)].blocks_light() {
                    break;
                }
                sky[cidx(x, y, z)] = MAX_LIGHT;
                queue.push_back((x, y, z, MAX_LIGHT));
            }
        }
    }
    while let Some((x, y, z, level)) = queue.pop_front() {
        for (dx, dy, dz) in [(1i32, 0i32, 0i32), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
            if !(0..SIZE as i32).contains(&nx)
                || !(0..WORLD_H as i32).contains(&ny)
                || !(0..SIZE as i32).contains(&nz)
            {
                continue;
            }
            let (nx, ny, nz) = (nx as usize, ny as usize, nz as usize);
            if blocks[cidx(nx, ny, nz)].blocks_light() {
                continue;
            }
            let candidate = if level == MAX_LIGHT && dy == -1 { MAX_LIGHT } else { level - 1 };
            if candidate > sky[cidx(nx, ny, nz)] {
                sky[cidx(nx, ny, nz)] = candidate;
                if candidate > 1 {
                    queue.push_back((nx, ny, nz, candidate));
                }
            }
        }
    }
    std::array::from_fn(|s| LightData::from_sky_slice(&sky[s * VOLUME..(s + 1) * VOLUME]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{STONE, TORCH, WATER};
    use crate::world::section::Section;

    /// 8 empty sections with `fill(x, y, z) -> Option<BlockId>` applied.
    fn column_with(fill: impl Fn(usize, i32, usize) -> Option<crate::world::block::BlockId>) -> Vec<Section> {
        let mut sections: Vec<Section> = (0..8).map(|_| Section::empty()).collect();
        for y in 0..256i32 {
            for x in 0..32usize {
                for z in 0..32usize {
                    if let Some(b) = fill(x, y, z) {
                        sections[(y / 32) as usize].set(x, (y % 32) as usize, z, b);
                    }
                }
            }
        }
        sections
    }

    fn sky_at(light: &[LightData; 8], x: usize, y: i32, z: usize) -> u8 {
        light[(y / 32) as usize].sky(x, (y % 32) as usize, z)
    }

    #[test]
    fn flat_ground_splits_sky_above_dark_below() {
        // Solid stone slab below y=20, open air above.
        let sections = column_with(|_, y, _| (y < 20).then_some(STONE));
        let light = light_new_column(&sections);
        assert_eq!(sky_at(&light, 5, 20, 5), 15, "first air cell above ground");
        assert_eq!(sky_at(&light, 5, 200, 5), 15, "high air");
        assert_eq!(sky_at(&light, 5, 10, 5), 0, "inside stone");
        assert!(matches!(light[7], LightData::Uniform(15)), "all-air section collapses to uniform");
        // Section 0 (y=0..31) has stone at y=0..19 and sky-lit air at y=20..31,
        // so it is Dense. Sky reads 0 inside the stone, 15 in the lit air.
        assert!(matches!(light[0], LightData::Dense(_)), "mixed section stays dense");
        assert_eq!(sky_at(&light, 5, 0, 5), 0, "deep stone cell is dark");
    }

    #[test]
    fn overhang_light_decrements_sideways_then_falls() {
        // Ground at y<20 plus a roof slab at y=40 covering x<16: under the
        // roof, light enters from the open side (x>=16) and decays inward.
        let sections = column_with(|x, y, _| {
            if y < 20 { return Some(STONE); }
            (y == 40 && x < 16).then_some(STONE)
        });
        let light = light_new_column(&sections);
        assert_eq!(sky_at(&light, 16, 30, 5), 15, "open shaft beside the roof");
        assert_eq!(sky_at(&light, 15, 30, 5), 14, "one step under the roof");
        assert_eq!(sky_at(&light, 12, 30, 5), 11, "four steps under the roof");
        assert_eq!(sky_at(&light, 0, 30, 5), 0, "16 steps in: fully dark (15-16 < 0)");
        // Horizontal entry happens at every y under the roof independently,
        // so every open cell under the roof at x=15 reads 14.
        assert_eq!(sky_at(&light, 15, 21, 5), 14);
    }

    #[test]
    fn sealed_cave_is_dark_and_water_blocks_sky() {
        // Stone up to y=100 with a sealed air pocket at y 40..44, x/z 10..14;
        // a water column at (20,*,20) from y=60..=100 over air below.
        let sections = column_with(|x, y, z| {
            let pocket = (40..44).contains(&y) && (10..14).contains(&x) && (10..14).contains(&z);
            let shaft = x == 20 && z == 20 && (0..=100).contains(&y);
            if pocket { return None; }
            if shaft { return ((60..=100).contains(&y)).then_some(WATER); }
            (y <= 100).then_some(STONE)
        });
        let light = light_new_column(&sections);
        assert_eq!(sky_at(&light, 11, 41, 11), 0, "sealed pocket gets no skylight");
        assert_eq!(sky_at(&light, 20, 50, 20), 0, "below the water plug: dark (water blocks light in M4)");
        assert_eq!(sky_at(&light, 20, 101, 20), 15, "above the water surface");
    }

    #[test]
    fn torch_block_does_not_block_the_sky_shaft() {
        // A floating torch at (5,50,5): the shaft below it stays sky-15.
        let sections = column_with(|x, y, z| (x == 5 && y == 50 && z == 5).then_some(TORCH));
        let light = light_new_column(&sections);
        assert_eq!(sky_at(&light, 5, 50, 5), 15, "the torch cell itself");
        assert_eq!(sky_at(&light, 5, 49, 5), 15, "below the torch");
    }

    #[test]
    fn generated_light_has_no_blocklight() {
        let sections = column_with(|_, y, _| (y < 20).then_some(STONE));
        let light = light_new_column(&sections);
        assert_eq!(light[1].block_light(5, 5, 5), 0);
        assert_eq!(light[6].block_light(5, 5, 5), 0);
    }

    #[test]
    fn dark_default_reads_zero_everywhere() {
        let l = LightData::dark();
        assert_eq!(l.sky(0, 0, 0), 0);
        assert_eq!(l.block_light(31, 31, 31), 0);
        assert_eq!(l.packed(15, 15, 15), 0);
    }

    #[test]
    fn uniform_stores_no_voxel_data_until_a_divergent_write() {
        let mut l = LightData::uniform(15, 0);
        assert!(matches!(l, LightData::Uniform(_)));
        assert!(!l.set_sky(4, 5, 6, 15), "writing the uniform value is a no-op");
        assert!(matches!(l, LightData::Uniform(_)), "no-op write must not promote");
        assert!(l.set_sky(4, 5, 6, 9), "divergent write reports a change");
        assert!(matches!(l, LightData::Dense(_)));
        assert_eq!(l.sky(4, 5, 6), 9);
        assert_eq!(l.sky(0, 0, 0), 15, "other voxels keep the old uniform value");
        assert_eq!(l.block_light(4, 5, 6), 0, "the other nibble is untouched");
    }

    #[test]
    fn channels_are_independent() {
        let mut l = LightData::dark();
        l.set_sky(1, 2, 3, 12);
        l.set_block_light(1, 2, 3, 7);
        assert_eq!(l.sky(1, 2, 3), 12);
        assert_eq!(l.block_light(1, 2, 3), 7);
        assert_eq!(l.packed(1, 2, 3), 12 | (7 << 4));
    }

    #[test]
    fn set_returns_whether_the_value_changed() {
        let mut l = LightData::dark();
        assert!(l.set_block_light(0, 0, 0, 5));
        assert!(!l.set_block_light(0, 0, 0, 5), "same value again: unchanged");
        assert!(l.set_block_light(0, 0, 0, 6));
    }

    #[test]
    #[allow(clippy::erasing_op, clippy::identity_op)]
    fn unpack_into_matches_pointwise_reads() {
        let mut l = LightData::uniform(15, 0);
        l.set_block_light(31, 0, 17, 14);
        let mut flat = vec![0u8; 32 * 32 * 32];
        l.unpack_into(&mut flat);
        // index formula: (y*32+z)*32+x — y=0 kept explicit for readability
        assert_eq!(flat[(0 * 32 + 17) * 32 + 31], 15 | (14 << 4));
        assert_eq!(flat[0], 15);
    }

    #[test]
    fn from_sky_slice_detects_uniformity() {
        let all15 = vec![15u8; 32768];
        assert!(matches!(LightData::from_sky_slice(&all15), LightData::Uniform(15)));
        let mut mixed = vec![15u8; 32768];
        mixed[100] = 3;
        let dense = LightData::from_sky_slice(&mixed);
        assert!(matches!(dense, LightData::Dense(_)));
        assert_eq!(dense.sky(4, 0, 3), 3, "voxel 100 = x4 z3 y0");
        assert_eq!(dense.block_light(4, 0, 3), 0, "sky slice seeds no blocklight");
    }
}
