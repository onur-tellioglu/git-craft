use bytemuck::{Pod, Zeroable};

/// Unpacked quad, CPU-side working representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quad {
    pub x: u32,        // 0..=33 (apron space)
    pub y: u32,
    pub z: u32,
    pub face: u32,     // 0..=5: +X -X +Y -Y +Z -Z
    pub w: u32,        // 1..=32, extent along the face's U axis
    pub h: u32,        // 1..=32, extent along the face's V axis
    pub ao: [u32; 4],  // 0..=3 per corner, order: (0,0) (w,0) (w,h) (0,h)
    pub skylight: u32, // 0..=15
    pub blocklight: u32,
    pub texture: u32,  // 0..=1023, texture array layer
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PackedQuad {
    pub data0: u32,
    pub data1: u32,
}

impl PackedQuad {
    pub fn pack(q: Quad) -> Self {
        debug_assert!(q.x < 34 && q.y < 34 && q.z < 34 && q.face < 6);
        debug_assert!((1..=32).contains(&q.w) && (1..=32).contains(&q.h));
        debug_assert!(q.skylight < 16 && q.blocklight < 16 && q.texture < 1024);
        let data0 = q.x | (q.y << 6) | (q.z << 12) | (q.face << 18) | ((q.w - 1) << 21);
        let ao = q.ao[0] | (q.ao[1] << 2) | (q.ao[2] << 4) | (q.ao[3] << 6);
        let data1 = (q.h - 1) | (ao << 5) | (q.skylight << 13) | (q.blocklight << 17) | (q.texture << 21);
        Self { data0, data1 }
    }

    pub fn unpack(self) -> Quad {
        let bits = |v: u32, off: u32, n: u32| (v >> off) & ((1 << n) - 1);
        let ao_bits = bits(self.data1, 5, 8);
        Quad {
            x: bits(self.data0, 0, 6),
            y: bits(self.data0, 6, 6),
            z: bits(self.data0, 12, 6),
            face: bits(self.data0, 18, 3),
            w: bits(self.data0, 21, 5) + 1,
            h: bits(self.data1, 0, 5) + 1,
            ao: [ao_bits & 3, (ao_bits >> 2) & 3, (ao_bits >> 4) & 3, (ao_bits >> 6) & 3],
            skylight: bits(self.data1, 13, 4),
            blocklight: bits(self.data1, 17, 4),
            texture: bits(self.data1, 21, 10),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(q: Quad) {
        assert_eq!(PackedQuad::pack(q).unpack(), q);
    }

    #[test]
    fn packs_and_unpacks_all_fields() {
        roundtrip(Quad {
            x: 12, y: 33, z: 7, face: 4, w: 32, h: 1,
            ao: [0, 1, 2, 3], skylight: 15, blocklight: 9, texture: 1000,
        });
    }

    #[test]
    fn packs_field_extremes() {
        roundtrip(Quad { x: 0, y: 0, z: 0, face: 0, w: 1, h: 1, ao: [0; 4], skylight: 0, blocklight: 0, texture: 0 });
        roundtrip(Quad { x: 33, y: 33, z: 33, face: 5, w: 32, h: 32, ao: [3; 4], skylight: 15, blocklight: 15, texture: 1023 });
    }

    #[test]
    fn packed_quad_is_8_bytes() {
        assert_eq!(std::mem::size_of::<PackedQuad>(), 8);
    }
}
