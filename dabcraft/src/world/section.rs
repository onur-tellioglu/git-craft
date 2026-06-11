use crate::world::block::{BlockId, AIR};

pub const SECTION_SIZE: usize = 32;
const VOLUME: usize = SECTION_SIZE * SECTION_SIZE * SECTION_SIZE;

/// Palette-compressed 32³ block storage (spec §4). `palette[0]` is the
/// uniform fill value; `bits == 0` means uniform (no voxel data at all).
/// Packed indices may span u64 word boundaries; VOLUME * bits is always a
/// multiple of 64, so a spanning entry's second word always exists.
#[derive(Clone, Debug)]
pub struct Section {
    palette: Vec<BlockId>,
    bits: u32,
    data: Vec<u64>,
}

fn bits_for(palette_len: usize) -> u32 {
    if palette_len <= 1 {
        0
    } else {
        usize::BITS - (palette_len - 1).leading_zeros()
    }
}

/// Read a packed index from a raw data buffer without borrowing the whole Section.
fn read_index_raw(data: &[u64], bits: u32, voxel: usize) -> usize {
    let bit = voxel * bits as usize;
    let (word, off) = (bit / 64, bit % 64);
    let mask = (1u64 << bits) - 1;
    let mut v = data[word] >> off;
    if off + bits as usize > 64 {
        v |= data[word + 1] << (64 - off);
    }
    (v & mask) as usize
}

impl Section {
    pub fn empty() -> Self {
        Self { palette: vec![AIR], bits: 0, data: Vec::new() }
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < SECTION_SIZE && y < SECTION_SIZE && z < SECTION_SIZE);
        (y * SECTION_SIZE + z) * SECTION_SIZE + x
    }

    fn read_index(&self, voxel: usize) -> usize {
        read_index_raw(&self.data, self.bits, voxel)
    }

    fn write_index(&mut self, voxel: usize, value: usize) {
        let bits = self.bits as usize;
        let bit = voxel * bits;
        let (word, off) = (bit / 64, bit % 64);
        let mask = (1u64 << bits) - 1;
        self.data[word] &= !(mask << off);
        self.data[word] |= (value as u64) << off;
        if off + bits > 64 {
            let spill = 64 - off;
            self.data[word + 1] &= !(mask >> spill);
            self.data[word + 1] |= (value as u64) >> spill;
        }
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockId {
        if self.bits == 0 {
            return self.palette[0];
        }
        self.palette[self.read_index(Self::index(x, y, z))]
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, block: BlockId) {
        let pi = match self.palette.iter().position(|&b| b == block) {
            Some(i) => i,
            None => {
                self.palette.push(block);
                let needed = bits_for(self.palette.len());
                if needed > self.bits {
                    self.repack(needed);
                }
                self.palette.len() - 1
            }
        };
        if self.bits == 0 {
            // Uniform and the value is already palette[0]: nothing to store.
            debug_assert_eq!(pi, 0);
            return;
        }
        self.write_index(Self::index(x, y, z), pi);
    }

    fn repack(&mut self, new_bits: u32) {
        let new_data_len = VOLUME * new_bits as usize / 64;
        if self.bits > 0 {
            let old_data = std::mem::take(&mut self.data);
            let old_bits = self.bits;
            self.bits = new_bits;
            self.data = vec![0u64; new_data_len];
            for voxel in 0..VOLUME {
                let idx = read_index_raw(&old_data, old_bits, voxel);
                self.write_index(voxel, idx);
            }
            return;
        }
        // Uniform → packed: all indices are 0 (palette[0]), already zeroed.
        self.bits = new_bits;
        self.data = vec![0u64; new_data_len];
    }

    /// Out-of-bounds counts as air. Kept for the M1 naive mesher; the greedy
    /// mesher reads neighbors through PaddedSection instead.
    pub fn get_or_air(&self, x: i32, y: i32, z: i32) -> BlockId {
        let r = 0..SECTION_SIZE as i32;
        if r.contains(&x) && r.contains(&y) && r.contains(&z) {
            self.get(x as usize, y as usize, z as usize)
        } else {
            AIR
        }
    }

    /// Some(block) when every voxel holds the same block.
    #[allow(dead_code)] // consumed by Task 12 (culling / early-out)
    pub fn uniform_block(&self) -> Option<BlockId> {
        if self.bits == 0 {
            return Some(self.palette[0]);
        }
        let first = self.read_index(0);
        (1..VOLUME).all(|v| self.read_index(v) == first).then(|| self.palette[first])
    }

    /// Bulk-decode all 32768 voxels into `out` (index = (y*32+z)*32+x).
    /// Hot path for padded-buffer fill.
    pub fn unpack_into(&self, out: &mut [BlockId]) {
        assert_eq!(out.len(), VOLUME);
        if self.bits == 0 {
            out.fill(self.palette[0]);
            return;
        }
        for (voxel, slot) in out.iter_mut().enumerate() {
            *slot = self.palette[self.read_index(voxel)];
        }
    }

    /// Rebuild the palette from live content, dropping orphaned entries and
    /// shrinking bit width. Call after worldgen finishes mutating a section.
    pub fn compact(&mut self) {
        let mut flat = vec![AIR; VOLUME];
        self.unpack_into(&mut flat);
        let fill = flat[0];
        let mut rebuilt = Section { palette: vec![fill], bits: 0, data: Vec::new() };
        for (voxel, &block) in flat.iter().enumerate() {
            if block != fill {
                let x = voxel % 32;
                let z = (voxel / 32) % 32;
                let y = voxel / 1024;
                rebuilt.set(x, y, z, block);
            }
        }
        *self = rebuilt;
    }

    #[allow(dead_code)] // consumed by Task 13 (mesh budget / F3 HUD diagnostics)
    pub fn palette_len(&self) -> usize {
        self.palette.len()
    }

    #[allow(dead_code)] // consumed by Task 13 (mesh budget / F3 HUD diagnostics)
    pub fn voxel_data_bytes(&self) -> usize {
        self.data.len() * 8
    }
}

impl PartialEq for Section {
    /// Semantic equality: same blocks at same positions, regardless of
    /// palette order, orphaned entries, or bit width.
    fn eq(&self, other: &Self) -> bool {
        let mut a = vec![AIR; VOLUME];
        let mut b = vec![AIR; VOLUME];
        self.unpack_into(&mut a);
        other.unpack_into(&mut b);
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{BlockId, AIR, DIRT, GRASS, STONE};

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

    #[test]
    fn empty_section_is_uniform_air_with_no_voxel_data() {
        let s = Section::empty();
        assert_eq!(s.uniform_block(), Some(AIR));
        assert_eq!(s.voxel_data_bytes(), 0);
    }

    #[test]
    fn palette_grows_through_repacks() {
        // 1→2 entries forces bits 0→1; 3 entries forces 1→2; 5 forces 2→3.
        let mut s = Section::empty();
        for (i, id) in (1..=8u16).enumerate() {
            s.set(i, 0, 0, BlockId(id));
        }
        for (i, id) in (1..=8u16).enumerate() {
            assert_eq!(s.get(i, 0, 0), BlockId(id), "voxel {i} after growth");
        }
        assert_eq!(s.get(20, 20, 20), AIR, "untouched voxels survive repacks");
        assert!(s.uniform_block().is_none());
    }

    #[test]
    fn every_voxel_roundtrips_with_word_spanning_indices() {
        // 3-bit indices: some voxel indices span u64 word boundaries.
        let mut s = Section::empty();
        let blocks = [AIR, GRASS, DIRT, STONE, BlockId(4)];
        for x in 0..32 {
            for y in 0..32 {
                for z in 0..32 {
                    s.set(x, y, z, blocks[(x * 7 + y * 3 + z) % 5]);
                }
            }
        }
        for x in 0..32 {
            for y in 0..32 {
                for z in 0..32 {
                    assert_eq!(s.get(x, y, z), blocks[(x * 7 + y * 3 + z) % 5]);
                }
            }
        }
    }

    #[test]
    fn compact_shrinks_palette_after_overwrites() {
        let mut s = Section::empty();
        for x in 0..32 {
            s.set(x, 0, 0, BlockId(x as u16 % 8));
        }
        for x in 0..32 {
            s.set(x, 0, 0, STONE); // orphan most palette entries
        }
        let before = s.palette_len();
        s.compact();
        assert!(s.palette_len() < before);
        assert_eq!(s.palette_len(), 2); // AIR (fill) + STONE
        for x in 0..32 {
            assert_eq!(s.get(x, 0, 0), STONE);
        }
    }

    #[test]
    fn semantic_equality_ignores_representation() {
        let mut a = Section::empty();
        let b = Section::empty();
        a.set(1, 2, 3, STONE);
        a.set(1, 2, 3, AIR); // a now has a bloated palette but identical content
        assert_eq!(a, b);
        let mut c = Section::empty();
        c.set(0, 0, 0, GRASS);
        assert_ne!(a, c);
    }

    #[test]
    fn unpack_into_matches_get() {
        let mut s = Section::empty();
        s.set(0, 0, 0, GRASS);
        s.set(31, 31, 31, STONE);
        let mut flat = vec![AIR; SECTION_SIZE * SECTION_SIZE * SECTION_SIZE];
        s.unpack_into(&mut flat);
        assert_eq!(flat[0], GRASS);
        assert_eq!(flat[(31 * SECTION_SIZE + 31) * SECTION_SIZE + 31], STONE);
        assert_eq!(flat[1], AIR);
    }
}
