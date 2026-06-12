// Per-section voxel light storage (spec §4): two 4-bit channels per voxel,
// skylight in the low nibble, blocklight in the high nibble.

pub const MAX_LIGHT: u8 = 15;

const SIZE: usize = 32;
const VOLUME: usize = SIZE * SIZE * SIZE;

/// Which flood-fill channel an operation targets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
// Variants are constructed by the M4 light engine (Tasks 4-6); the chunk
// accessors already match on the channel.
#[allow(dead_code)]
pub enum LightChannel {
    Sky,
    Block,
}

#[allow(dead_code)] // consumed by the M4 light engine (Tasks 3-5)
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

#[allow(dead_code)] // consumed by the M4 light engine (Tasks 3-5)
impl LightData {
    pub fn dark() -> Self {
        LightData::Uniform(0)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

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
