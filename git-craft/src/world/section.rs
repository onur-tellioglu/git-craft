use crate::world::block::{AIR, BlockId};

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

/// Take a fixed-size chunk from `bytes` at cursor `c`, advancing it. None on
/// truncation. The building block for the little-endian readers below.
fn take<const N: usize>(bytes: &[u8], c: &mut usize) -> Option<[u8; N]> {
    let end = c.checked_add(N)?;
    let slice = bytes.get(*c..end)?;
    *c = end;
    Some(slice.try_into().expect("slice length checked above"))
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
        Self {
            palette: vec![AIR],
            bits: 0,
            data: Vec::new(),
        }
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

    /// Some(block) when every voxel holds the same block.
    /// Reserved for M4 cave culling / mesher fast path.
    #[allow(dead_code)]
    pub fn uniform_block(&self) -> Option<BlockId> {
        if self.bits == 0 {
            return Some(self.palette[0]);
        }
        let first = self.read_index(0);
        (1..VOLUME)
            .all(|v| self.read_index(v) == first)
            .then(|| self.palette[first])
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
        let mut rebuilt = Section {
            palette: vec![fill],
            bits: 0,
            data: Vec::new(),
        };
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

    /// Append this section's bytes (palette form) to `out`, for persistence.
    /// Layout: `[palette_len u16][palette × u16][bits u8][data × u64]`, all
    /// little-endian. The data word count is derivable from `bits`, so it is
    /// not stored. See [`Section::read_bytes`] for the inverse.
    pub fn write_bytes(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&(self.palette.len() as u16).to_le_bytes());
        for b in &self.palette {
            out.extend_from_slice(&b.0.to_le_bytes());
        }
        out.push(self.bits as u8);
        for word in &self.data {
            out.extend_from_slice(&word.to_le_bytes());
        }
    }

    /// Parse a section written by [`Section::write_bytes`]. Returns the section
    /// and the number of bytes consumed (so columns can pack sections
    /// back-to-back). `None` on truncation or an invalid header — a corrupt
    /// save degrades to regeneration rather than a panic.
    pub fn read_bytes(bytes: &[u8]) -> Option<(Section, usize)> {
        let mut c = 0usize;
        let palette_len = u16::from_le_bytes(take::<2>(bytes, &mut c)?) as usize;
        if palette_len == 0 {
            return None;
        }
        let mut palette = Vec::with_capacity(palette_len);
        for _ in 0..palette_len {
            palette.push(BlockId(u16::from_le_bytes(take::<2>(bytes, &mut c)?)));
        }
        let bits = take::<1>(bytes, &mut c)?[0] as u32;
        if bits > 16 || (bits == 0 && palette_len != 1) {
            return None;
        }
        let words = if bits == 0 {
            0
        } else {
            VOLUME * bits as usize / 64
        };
        let mut data = Vec::with_capacity(words);
        for _ in 0..words {
            data.push(u64::from_le_bytes(take::<8>(bytes, &mut c)?));
        }
        // Validate that every packed palette index is in range. An out-of-range
        // index would panic in unpack_into/get, killing the worker thread and
        // silently discarding all subsequent saves. Reject the section so
        // deserialization fails cleanly → the worker returns Loaded::Failed →
        // the column regenerates (the stated design contract).
        if bits > 0 {
            for voxel in 0..VOLUME {
                if read_index_raw(&data, bits, voxel) >= palette_len {
                    return None;
                }
            }
        }
        Some((
            Section {
                palette,
                bits,
                data,
            },
            c,
        ))
    }

    #[cfg(test)]
    pub fn palette_len(&self) -> usize {
        self.palette.len()
    }

    #[cfg(test)]
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
    use crate::world::block::{AIR, BlockId, DIRT, GRASS, STONE};

    #[test]
    fn set_then_get_roundtrips() {
        let mut s = Section::empty();
        s.set(31, 0, 17, STONE);
        assert_eq!(s.get(31, 0, 17), STONE);
        assert_eq!(s.get(0, 0, 0), AIR);
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

    fn roundtrip(s: &Section) -> Section {
        let mut buf = Vec::new();
        s.write_bytes(&mut buf);
        let (back, consumed) = Section::read_bytes(&buf).expect("valid section bytes");
        assert_eq!(
            consumed,
            buf.len(),
            "read_bytes must consume the whole blob"
        );
        back
    }

    #[test]
    fn serialize_roundtrips_empty_and_uniform() {
        assert_eq!(roundtrip(&Section::empty()), Section::empty());
        let mut uniform = Section::empty();
        uniform.set(5, 6, 7, STONE);
        uniform.set(5, 6, 7, AIR); // back to uniform-air content
        assert_eq!(roundtrip(&uniform), uniform);
    }

    #[test]
    fn serialize_roundtrips_multi_palette_and_word_spanning() {
        let mut s = Section::empty();
        let blocks = [AIR, GRASS, DIRT, STONE, BlockId(4)]; // 5 entries → 3-bit indices
        for x in 0..32 {
            for y in 0..32 {
                for z in 0..32 {
                    s.set(x, y, z, blocks[(x * 7 + y * 3 + z) % 5]);
                }
            }
        }
        assert_eq!(roundtrip(&s), s);
    }

    #[test]
    fn read_bytes_rejects_truncated_input() {
        let mut s = Section::empty();
        s.set(0, 0, 0, GRASS);
        let mut buf = Vec::new();
        s.write_bytes(&mut buf);
        assert!(Section::read_bytes(&buf[..buf.len() - 1]).is_none());
        assert!(Section::read_bytes(&[]).is_none());
    }

    #[test]
    fn two_sections_pack_back_to_back() {
        let mut a = Section::empty();
        a.set(1, 2, 3, STONE);
        let mut b = Section::empty();
        b.set(4, 5, 6, GRASS);
        let mut buf = Vec::new();
        a.write_bytes(&mut buf);
        b.write_bytes(&mut buf);
        let (ra, na) = Section::read_bytes(&buf).unwrap();
        let (rb, nb) = Section::read_bytes(&buf[na..]).unwrap();
        assert_eq!(ra, a);
        assert_eq!(rb, b);
        assert_eq!(na + nb, buf.len());
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

    /// A corrupt blob where palette_len=1 but bits=2 and every data word is
    /// 0xFFFFFFFFFFFFFFFF (packed index 3, out of range) must be rejected rather
    /// than returned as a valid Section that would panic on unpack.
    #[test]
    fn read_bytes_rejects_out_of_range_palette_indices() {
        let mut buf = Vec::new();
        // palette_len = 1 (AIR only)
        buf.extend_from_slice(&1u16.to_le_bytes());
        // palette entry: AIR = BlockId(0)
        buf.extend_from_slice(&0u16.to_le_bytes());
        // bits = 2 (allows indices 0..3, but palette only has index 0)
        buf.push(2u8);
        // data words: VOLUME * bits / 64 = 32768 * 2 / 64 = 1024 words,
        // all 0xFF → every 2-bit field encodes index 3, which is out of range.
        let words = VOLUME * 2 / 64;
        for _ in 0..words {
            buf.extend_from_slice(&u64::MAX.to_le_bytes());
        }
        assert!(
            Section::read_bytes(&buf).is_none(),
            "out-of-range packed index must cause read_bytes to return None"
        );
    }
}
