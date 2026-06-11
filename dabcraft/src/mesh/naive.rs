use crate::mesh::quad::{PackedQuad, Quad};
use crate::world::section::{Section, SECTION_SIZE};

/// Face order matches the packed format: +X -X +Y -Y +Z -Z.
const NEIGHBOR_OFFSETS: [(i32, i32, i32); 6] =
    [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];

/// One 1x1 quad per exposed face. Temporary M1 mesher; M2 replaces it
/// with binary greedy meshing behind the same output contract.
pub fn mesh_naive(section: &Section) -> Vec<PackedQuad> {
    let mut quads = Vec::new();
    for y in 0..SECTION_SIZE {
        for z in 0..SECTION_SIZE {
            for x in 0..SECTION_SIZE {
                let block = section.get(x, y, z);
                if !block.is_solid() {
                    continue;
                }
                for (face, (dx, dy, dz)) in NEIGHBOR_OFFSETS.iter().enumerate() {
                    let neighbor = section.get_or_air(x as i32 + dx, y as i32 + dy, z as i32 + dz);
                    if neighbor.is_solid() {
                        continue;
                    }
                    quads.push(PackedQuad::pack(Quad {
                        x: x as u32,
                        y: y as u32,
                        z: z as u32,
                        face: face as u32,
                        w: 1,
                        h: 1,
                        ao: [3; 4],        // real AO arrives in M2 with the apron
                        skylight: 15,      // real flood-fill light arrives in M4
                        blocklight: 0,
                        texture: block.0 as u32,
                        flip: 0,
                    }));
                }
            }
        }
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::STONE;
    use crate::world::section::Section;

    #[test]
    fn empty_section_yields_no_quads() {
        assert!(mesh_naive(&Section::empty()).is_empty());
    }

    #[test]
    fn single_block_yields_six_quads() {
        let mut s = Section::empty();
        s.set(5, 5, 5, STONE);
        assert_eq!(mesh_naive(&s).len(), 6);
    }

    #[test]
    fn touching_faces_are_culled() {
        let mut s = Section::empty();
        s.set(5, 5, 5, STONE);
        s.set(6, 5, 5, STONE);
        // 12 faces total, 2 shared (hidden) => 10
        assert_eq!(mesh_naive(&s).len(), 10);
    }

    #[test]
    fn full_floor_slab_face_count() {
        let mut s = Section::empty();
        for x in 0..32 {
            for z in 0..32 {
                s.set(x, 0, z, STONE);
            }
        }
        // top 1024 + bottom 1024 + 4 sides * 32 = 2176
        assert_eq!(mesh_naive(&s).len(), 2176);
    }

    #[test]
    fn quads_carry_block_id_as_texture() {
        let mut s = Section::empty();
        s.set(0, 0, 0, STONE);
        let quads = mesh_naive(&s);
        assert!(quads.iter().all(|q| q.unpack().texture == STONE.0 as u32));
    }
}
