// Binary greedy mesher (git-craft spec §5, M2 Tasks 5–6).
// Per-corner AO with merge-safe 9-bit neighborhood keys and diagonal flip.

use std::collections::HashMap;

use crate::mesh::padded::{PADDED, PaddedSection};
use crate::mesh::quad::{PackedQuad, Quad};
use crate::world::block::{BlockId, WATER};

const SIZE: usize = 32;

/// face for (axis, direction): FACE_OF[axis][0]=+dir, [1]=-dir.
const FACE_OF: [[u32; 2]; 3] = [[2, 3], [0, 1], [4, 5]];

/// Integer mirrors of the shader's FACE tables (cross(U,V) = outward normal).
const FACE_N: [[i32; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];
const FACE_U: [[i32; 3]; 6] = [
    [0, 1, 0],
    [0, 0, 1],
    [0, 0, 1],
    [1, 0, 0],
    [1, 0, 0],
    [0, 1, 0],
];
const FACE_V: [[i32; 3]; 6] = [
    [0, 0, 1],
    [0, 1, 0],
    [1, 0, 0],
    [0, 0, 1],
    [0, 1, 0],
    [1, 0, 0],
];

/// Binary greedy mesher (spec §5). Reusable: `mesh` clears prior state.
/// M2: per-corner AO (Task 6), flip bit
/// M4: flood-fill light baked into quads (Task 7).
pub struct Mesher {
    /// Solidity columns. Bit c of axis_cols[axis][i][j] = solid at padded
    /// coord c along the axis. axis 0: bits=Y,i=z,j=x; 1: bits=X,i=y,j=z;
    /// 2: bits=Z,i=y,j=x.
    axis_cols: [[[u64; PADDED]; PADDED]; 3],
    /// (face,slice,block,ao,light) key → 32×32 face bit-plane.
    planes: HashMap<u64, [u32; SIZE]>,
    quads: Vec<PackedQuad>,
}

/// Bits 0..16: block id, 16..25: ao_key (9 bits), 25..31: slice (6 bits),
/// 31..34: face (3 bits), 34..42: light packed byte.
fn plane_key(face: u32, slice: u32, block: u16, ao_key: u32, light: u8) -> u64 {
    debug_assert!(
        ao_key < 512,
        "ao_key {ao_key} exceeds 9 bits; would corrupt slice field"
    );
    block as u64
        | (ao_key as u64) << 16
        | (slice as u64) << 25
        | (face as u64) << 31
        | (light as u64) << 34
}

/// 9-bit solidity pattern of the out-layer 3×3 neighborhood of a face,
/// in the face's U/V axes. Bit (du+1)*3+(dv+1); (px,py,pz) are the padded
/// coords of the solid cell owning the face.
fn ao_neighborhood(
    padded: &PaddedSection,
    px: usize,
    py: usize,
    pz: usize,
    face: usize,
    occ: &impl Fn(BlockId) -> bool,
) -> u32 {
    let n = FACE_N[face];
    let (u, v) = (FACE_U[face], FACE_V[face]);
    let base = [px as i32 + n[0], py as i32 + n[1], pz as i32 + n[2]];
    let mut key = 0u32;
    for du in -1..=1i32 {
        for dv in -1..=1i32 {
            let q = [
                base[0] + du * u[0] + dv * v[0],
                base[1] + du * u[1] + dv * v[1],
                base[2] + du * u[2] + dv * v[2],
            ];
            // The normal step keeps exactly one coordinate at the apron rim;
            // U/V steps move the other two, which started in 1..=32. All
            // three therefore stay inside 0..34.
            if occ(padded.get(q[0] as usize, q[1] as usize, q[2] as usize)) {
                key |= 1 << ((du + 1) * 3 + (dv + 1));
            }
        }
    }
    key
}

/// Corner rule (spec §5): side1 && side2 → 0, else 3-(side1+side2+corner).
/// Corner order (0,0) (w,0) (w,h) (0,h) in face U/V space.
fn corner_ao(ao_key: u32) -> [u32; 4] {
    let bit = |i: u32| (ao_key >> i) & 1;
    let rule = |s1: u32, s2: u32, c: u32| {
        if s1 == 1 && s2 == 1 {
            0
        } else {
            3 - (s1 + s2 + c)
        }
    };
    [
        rule(bit(1), bit(3), bit(0)),
        rule(bit(7), bit(3), bit(6)),
        rule(bit(7), bit(5), bit(8)),
        rule(bit(1), bit(5), bit(2)),
    ]
}

impl Mesher {
    pub fn new() -> Self {
        Self {
            axis_cols: [[[0; PADDED]; PADDED]; 3],
            planes: HashMap::with_capacity(256),
            quads: Vec::new(),
        }
    }

    /// Mesh treating every non-air block as one opaque solid (the M2–M4
    /// behavior). The production path uses `mesh_layers`; this stays as the
    /// reference single-layer mesher and the basis for the mesher unit tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn mesh(&mut self, padded: &PaddedSection) -> Vec<PackedQuad> {
        self.mesh_with(padded, |b| b.is_solid(), |_| true)
    }

    /// Split into (opaque, water) layers for the transparent water pass.
    /// Opaque treats water as empty so the seafloor face (solid→water) is
    /// emitted and visible through the surface; water keeps only WATER-owned
    /// faces against air (water→solid stays culled, water→water internal).
    pub fn mesh_layers(&mut self, padded: &PaddedSection) -> (Vec<PackedQuad>, Vec<PackedQuad>) {
        let opaque = self.mesh_with(padded, |b| b.is_solid() && b != WATER, |_| true);
        let water = self.mesh_with(padded, |b| b.is_solid(), |b| b == WATER);
        (opaque, water)
    }

    /// Core mesh pass. `occ` is the solidity used for face culling + AO; `keep`
    /// filters which owning blocks emit a face (so one occupancy can yield a
    /// single block type's surface).
    fn mesh_with(
        &mut self,
        padded: &PaddedSection,
        occ: impl Fn(BlockId) -> bool,
        keep: impl Fn(BlockId) -> bool,
    ) -> Vec<PackedQuad> {
        self.axis_cols = [[[0; PADDED]; PADDED]; 3];
        self.planes.clear();
        self.quads.clear();
        self.build_axis_cols(padded, &occ);
        self.build_planes(padded, &occ, &keep);
        self.sweep_planes();
        std::mem::take(&mut self.quads)
    }

    fn build_axis_cols(&mut self, padded: &PaddedSection, occ: &impl Fn(BlockId) -> bool) {
        for y in 0..PADDED {
            for z in 0..PADDED {
                for x in 0..PADDED {
                    if occ(padded.get(x, y, z)) {
                        self.axis_cols[0][z][x] |= 1 << y;
                        self.axis_cols[1][y][z] |= 1 << x;
                        self.axis_cols[2][y][x] |= 1 << z;
                    }
                }
            }
        }
    }

    fn build_planes(
        &mut self,
        padded: &PaddedSection,
        occ: &impl Fn(BlockId) -> bool,
        keep: &impl Fn(BlockId) -> bool,
    ) {
        #[allow(clippy::needless_range_loop)]
        // axis indexes axis_cols AND FACE_OF; iterator form is less clear
        for axis in 0..3usize {
            for i in 1..=SIZE {
                for j in 1..=SIZE {
                    let col = self.axis_cols[axis][i][j];
                    // +dir: solid at c, air at c+1.
                    // >>1 drops the lower apron bit; the u32 cast truncates
                    // the upper one — only interior faces survive.
                    let pos = ((col & !(col >> 1)) >> 1) as u32;
                    let neg = ((col & !(col << 1)) >> 1) as u32;
                    for (dir, mut mask) in [(0usize, pos), (1usize, neg)] {
                        let face = FACE_OF[axis][dir];
                        while mask != 0 {
                            let c = mask.trailing_zeros();
                            mask &= mask - 1;
                            let (x, y, z) = match axis {
                                0 => (j, (c + 1) as usize, i),
                                1 => ((c + 1) as usize, i, j),
                                _ => (j, i, (c + 1) as usize),
                            };
                            let block = padded.get(x, y, z);
                            if !keep(block) {
                                continue;
                            }
                            let ao_key = ao_neighborhood(padded, x, y, z, face as usize, occ);
                            let n = FACE_N[face as usize];
                            let light = padded.light_packed(
                                (x as i32 + n[0]) as usize,
                                (y as i32 + n[1]) as usize,
                                (z as i32 + n[2]) as usize,
                            );
                            let key = plane_key(face, c, block.0, ao_key, light);
                            self.planes.entry(key).or_insert([0u32; SIZE])[i - 1] |= 1 << (j - 1);
                        }
                    }
                }
            }
        }
    }

    fn sweep_planes(&mut self) {
        let planes = std::mem::take(&mut self.planes);
        for (key, plane) in planes {
            let block = (key & 0xFFFF) as u16;
            let ao_key = ((key >> 16) & 0x1FF) as u32;
            let slice = ((key >> 25) & 0x3F) as u32;
            let face = ((key >> 31) & 0x7) as u32;
            let light = ((key >> 34) & 0xFF) as u8;
            sweep_plane(face, slice, block, ao_key, light, plane, &mut self.quads);
        }
    }
}

impl Default for Mesher {
    fn default() -> Self {
        Self::new()
    }
}

/// Greedy rectangle decomposition of one 32×32 bit plane.
fn sweep_plane(
    face: u32,
    slice: u32,
    block: u16,
    ao_key: u32,
    light: u8,
    mut plane: [u32; SIZE],
    out: &mut Vec<PackedQuad>,
) {
    for row in 0..SIZE {
        let mut b = 0u32;
        while b < SIZE as u32 {
            b += (plane[row] >> b).trailing_zeros();
            if b >= SIZE as u32 {
                break;
            }
            let rb = (plane[row] >> b).trailing_ones();
            let run_mask = u32::checked_shl(1, rb).map_or(!0u32, |v| v - 1);
            let mask = run_mask << b;
            let mut rw = 1usize;
            while row + rw < SIZE {
                if (plane[row + rw] >> b) & run_mask != run_mask {
                    break;
                }
                plane[row + rw] &= !mask;
                rw += 1;
            }
            emit(
                face, slice, row as u32, b, rw as u32, rb, block, ao_key, light, out,
            );
            b += rb;
        }
    }
}

#[allow(clippy::too_many_arguments)] // internal plumbing of one algorithm step
fn emit(
    face: u32,
    slice: u32,
    row: u32,
    bit: u32,
    rw: u32,
    rb: u32,
    block: u16,
    ao_key: u32,
    light: u8,
    out: &mut Vec<PackedQuad>,
) {
    // See the face/plane mapping table; w spans U, h spans V.
    let ((x, y, z), w, h) = match face {
        0 => ((slice, row, bit), rw, rb),
        1 => ((slice, row, bit), rb, rw),
        2 => ((bit, slice, row), rw, rb),
        3 => ((bit, slice, row), rb, rw),
        4 => ((bit, row, slice), rb, rw),
        _ => ((bit, row, slice), rw, rb),
    };
    let ao = corner_ao(ao_key);
    let flip = u32::from(ao[0] + ao[2] > ao[1] + ao[3]);
    out.push(PackedQuad::pack(Quad {
        x,
        y,
        z,
        face,
        w,
        h,
        ao,
        skylight: (light & 0x0F) as u32,
        blocklight: (light >> 4) as u32,
        texture: block as u32,
        flip,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::padded::PaddedSection;
    use crate::mesh::quad::Quad;
    use crate::world::block::{DIRT, GRASS, STONE};

    fn mesh(p: &PaddedSection) -> Vec<Quad> {
        Mesher::new().mesh(p).iter().map(|pq| pq.unpack()).collect()
    }

    #[test]
    fn empty_section_emits_nothing() {
        assert!(mesh(&PaddedSection::air()).is_empty());
    }

    #[test]
    fn plain_mesh_still_treats_water_as_a_solid() {
        use crate::world::block::WATER;
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, WATER);
        assert_eq!(mesh(&p).len(), 6, "mesh() keeps the M2 all-solid behavior");
    }

    #[test]
    fn water_splits_into_its_own_layer_with_a_visible_seafloor() {
        use crate::world::block::{SAND, WATER};
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, SAND); // seafloor, interior (5,5,5)
        p.set(6, 7, 6, WATER); // water directly above
        let (opaque, water) = Mesher::new().mesh_layers(&p);
        let op: Vec<Quad> = opaque.iter().map(|q| q.unpack()).collect();
        let wq: Vec<Quad> = water.iter().map(|q| q.unpack()).collect();
        // Opaque keeps the sand AND emits its top face (water counted as empty).
        assert!(
            op.iter().any(|q| q.texture == SAND.0 as u32 && q.face == 2),
            "seafloor top face must be emitted into the opaque layer"
        );
        assert!(
            op.iter().all(|q| q.texture != WATER.0 as u32),
            "opaque layer holds no water"
        );
        // Water layer is only WATER, has a top surface, and no bottom (culled by the sand).
        assert!(!wq.is_empty() && wq.iter().all(|q| q.texture == WATER.0 as u32));
        assert!(wq.iter().any(|q| q.face == 2), "water top surface present");
        assert!(
            wq.iter().all(|q| q.face != 3),
            "water bottom against the sand is culled"
        );
    }

    #[test]
    fn submerged_water_block_emits_no_water_faces() {
        use crate::world::block::{STONE, WATER};
        // Water fully boxed in by stone: no water→air faces anywhere.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, WATER);
        for (dx, dy, dz) in [
            (1, 0, 0),
            (-1i32, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ] {
            p.set(
                (6 + dx) as usize,
                (6 + dy) as usize,
                (6 + dz) as usize,
                STONE,
            );
        }
        let (_, water) = Mesher::new().mesh_layers(&p);
        assert!(water.is_empty(), "enclosed water has no visible surface");
    }

    #[test]
    fn single_block_emits_six_unit_quads() {
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE); // padded coords; interior (5,5,5)
        let quads = mesh(&p);
        assert_eq!(quads.len(), 6);
        let mut faces: Vec<u32> = quads.iter().map(|q| q.face).collect();
        faces.sort_unstable();
        assert_eq!(faces, vec![0, 1, 2, 3, 4, 5]);
        for q in &quads {
            assert_eq!((q.w, q.h), (1, 1));
            assert_eq!(
                (q.x, q.y, q.z),
                (5, 5, 5),
                "interior coords, face {}",
                q.face
            );
            assert_eq!(q.texture, STONE.0 as u32);
            assert_eq!(q.skylight, 15);
            assert_eq!(q.blocklight, 0);
        }
    }

    #[test]
    fn flat_slab_merges_to_one_quad_per_side() {
        // Full 32×32 floor, 1 thick: 6 quads total (the M1 naive mesher made 2176).
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for z in 1..=32 {
                p.set(x, 1, z, GRASS);
            }
        }
        let quads = mesh(&p);
        assert_eq!(quads.len(), 6);
        let top = quads.iter().find(|q| q.face == 2).unwrap();
        assert_eq!((top.w, top.h), (32, 32));
        assert_eq!((top.x, top.y, top.z), (0, 0, 0));
    }

    #[test]
    fn different_blocks_do_not_merge() {
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, GRASS);
        p.set(7, 6, 6, DIRT);
        let quads = mesh(&p);
        let tops: Vec<_> = quads.iter().filter(|q| q.face == 2).collect();
        assert_eq!(tops.len(), 2);
        assert!(tops.iter().all(|q| (q.w, q.h) == (1, 1)));
    }

    #[test]
    fn solid_apron_culls_boundary_faces() {
        // Floor at interior y=0 with solid apron below: no -Y faces emitted.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for z in 1..=32 {
                p.set(x, 1, z, STONE); // interior floor
                p.set(x, 0, z, STONE); // apron below (neighbor section's top)
            }
        }
        let quads = mesh(&p);
        assert!(
            quads.iter().all(|q| q.face != 3),
            "bottom faces must be culled by the apron"
        );
        assert_eq!(quads.iter().filter(|q| q.face == 2).count(), 1);
    }

    #[test]
    fn apron_never_emits_its_own_faces() {
        // Solid apron slab, empty interior: zero quads.
        let mut p = PaddedSection::air();
        for x in 0..34 {
            for z in 0..34 {
                p.set(x, 0, z, STONE);
            }
        }
        assert!(mesh(&p).is_empty());
    }

    #[test]
    fn interior_buried_voxels_emit_nothing() {
        // 3×3×3 solid cube: only the 6 outer 3×3 faces appear, 1 quad each.
        let mut p = PaddedSection::air();
        for x in 10..13 {
            for y in 10..13 {
                for z in 10..13 {
                    p.set(x, y, z, STONE);
                }
            }
        }
        let quads = mesh(&p);
        assert_eq!(quads.len(), 6);
        assert!(quads.iter().all(|q| (q.w, q.h) == (3, 3)));
    }

    #[test]
    fn l_shape_merges_greedily() {
        // Top face of an L: row z=5 has x∈{5,6,7}; row z=6 has x=5 (interior).
        let mut p = PaddedSection::air();
        for x in 6..9 {
            p.set(x, 6, 6, STONE);
        }
        p.set(6, 6, 7, STONE);
        let quads = mesh(&p);
        let tops: Vec<_> = quads.iter().filter(|q| q.face == 2).collect();
        let covered: u32 = tops.iter().map(|q| q.w * q.h).sum();
        assert_eq!(covered, 4, "top quads must tile the L exactly");
        assert_eq!(tops.len(), 2);
    }

    #[test]
    fn isolated_block_has_full_ao() {
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        for q in mesh(&p) {
            assert_eq!(q.ao, [3, 3, 3, 3], "face {}", q.face);
            assert_eq!(q.flip, 0);
        }
    }

    #[test]
    fn wall_darkens_adjacent_top_corners() {
        // Ground at interior (5,5,5); wall at interior (6,6,5) — +X and one up.
        // Ground's top face (+Y): U=Z, V=X; the wall sits at (du=0, dv=+1)
        // → side bit 5 → darkens corners 2=(w,h) and 3=(0,h) by one each.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE); // ground, padded coords
        p.set(7, 7, 6, STONE); // wall above-right (padded)
        let quads = mesh(&p);
        let top = quads
            .iter()
            .find(|q| q.face == 2 && (q.x, q.y, q.z) == (5, 5, 5))
            .unwrap();
        assert_eq!(top.ao, [3, 3, 2, 2]);
    }

    #[test]
    fn diagonal_only_neighbor_gives_corner_ao_two() {
        // Only a diagonal block above the top face's out-layer: corner bit
        // only → ao 2 at that corner. Block at padded (7,7,7) is offset
        // (+1 X, +1 Z) from the out-layer center (6,7,6): du=+1 (U=Z),
        // dv=+1 (V=X) → bit 8 → corner 2=(w,h) only.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(7, 7, 7, STONE);
        let quads = mesh(&p);
        let top = quads
            .iter()
            .find(|q| q.face == 2 && (q.x, q.y, q.z) == (5, 5, 5))
            .unwrap();
        assert_eq!(top.ao, [3, 3, 2, 3]);
    }

    #[test]
    fn fully_cornered_cell_gets_ao_zero() {
        // Two perpendicular walls above the ground block: side1 && side2 → 0.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE); // ground
        p.set(7, 7, 6, STONE); // +V wall (x+1 at out-layer height)
        p.set(6, 7, 7, STONE); // +U wall (z+1 at out-layer height)
        let top = mesh(&p)
            .into_iter()
            .find(|q| q.face == 2 && (q.x, q.y, q.z) == (5, 5, 5))
            .unwrap();
        assert_eq!(top.ao[2], 0, "corner (w,h) boxed in by both walls");
    }

    #[test]
    fn ao_boundary_splits_merge() {
        // 2-block strip along z; a wall darkens only one cell's neighborhood
        // → the top face must NOT merge into a single quad.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(6, 6, 7, STONE);
        p.set(7, 7, 6, STONE); // wall darkening only the first cell
        let quads = mesh(&p);
        // Filter to the ground level: the helper wall emits its own top face.
        let tops: Vec<_> = quads.iter().filter(|q| q.face == 2 && q.y == 5).collect();
        assert_eq!(tops.len(), 2, "AO difference must split the merge");
    }

    #[test]
    fn anisotropic_ao_sets_flip_bit() {
        // Darken corners 1=(w,0) and 3=(0,h) via two opposite diagonal
        // blocks → ao [3,2,3,2] → ao0+ao2=6 > ao1+ao3=4 → flip.
        // Top face U=Z, V=X. Corner 1 ← (du=+1, dv=−1) = out-layer +z,−x:
        // padded (5,7,7). Corner 3 ← (du=−1, dv=+1) = −z,+x: padded (7,7,5).
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(5, 7, 7, STONE);
        p.set(7, 7, 5, STONE);
        let top = mesh(&p)
            .into_iter()
            .find(|q| q.face == 2 && (q.x, q.y, q.z) == (5, 5, 5))
            .unwrap();
        assert_eq!(top.ao, [3, 2, 3, 2]);
        assert_eq!(top.flip, 1, "3+3 > 2+2 must flip the diagonal");
    }

    #[test]
    fn slab_interior_still_merges_fully() {
        // AO keys are uniform across a flat slab (edge cells see the all-air
        // apron identically), so the whole top still merges to one quad.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for z in 1..=32 {
                p.set(x, 1, z, GRASS);
            }
        }
        let quads = mesh(&p);
        assert_eq!(quads.iter().filter(|q| q.face == 2).count(), 1);
    }

    #[test]
    fn x_face_wh_orientation_is_not_transposed() {
        // A slab 1 deep in X, 3 tall in Y, 2 wide in Z. Pins w/h independently
        // on the X faces: a rw/rb transposition in emit() keeps the area (all
        // other tests pass) but shears the quad once backface culling lands.
        let mut p = PaddedSection::air();
        for y in 1..=3 {
            for z in 1..=2 {
                p.set(5, y, z, STONE);
            }
        }
        let quads = mesh(&p);
        let pos_x = quads.iter().find(|q| q.face == 0).unwrap();
        assert_eq!(
            (pos_x.w, pos_x.h),
            (3, 2),
            "+X face: w spans Y(U), h spans Z(V)"
        );
        let neg_x = quads.iter().find(|q| q.face == 1).unwrap();
        assert_eq!(
            (neg_x.w, neg_x.h),
            (2, 3),
            "-X face: w spans Z(U), h spans Y(V)"
        );
    }

    #[test]
    fn neg_u_side_darkens_corners_0_and_3() {
        // Top face (+Y): U=Z, V=X. Wall at out-layer du=-1 (z-1) → bit 1 →
        // side1 of corners 0=(0,0) and 3=(0,h).
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(6, 7, 5, STONE); // x unchanged, z-1 above the out-layer
        let top = mesh(&p)
            .into_iter()
            .find(|q| q.face == 2 && (q.x, q.y, q.z) == (5, 5, 5))
            .unwrap();
        assert_eq!(top.ao, [2, 3, 3, 2], "bit 1: corners 0 and 3 darkened");
    }

    #[test]
    fn neg_v_side_darkens_corners_0_and_1() {
        // Top face (+Y): V=X. Wall at out-layer dv=-1 (x-1) → bit 3 →
        // side2 of corners 0=(0,0) and 1=(w,0).
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(5, 7, 6, STONE); // x-1, z unchanged
        let top = mesh(&p)
            .into_iter()
            .find(|q| q.face == 2 && (q.x, q.y, q.z) == (5, 5, 5))
            .unwrap();
        assert_eq!(top.ao, [2, 2, 3, 3], "bit 3: corners 0 and 1 darkened");
    }

    #[test]
    fn neg_u_neg_v_diagonal_darkens_corner_0_only() {
        // Out-layer (du=-1, dv=-1) → bit 0, the corner bit of corner 0.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(5, 7, 5, STONE); // x-1 and z-1
        let top = mesh(&p)
            .into_iter()
            .find(|q| q.face == 2 && (q.x, q.y, q.z) == (5, 5, 5))
            .unwrap();
        assert_eq!(top.ao, [2, 3, 3, 3], "bit 0: only corner 0 darkened");
    }

    // --- M4 Task 7: light tests ---

    #[test]
    fn quad_light_samples_the_out_cell_not_the_block() {
        // Stone at padded (6,6,6); the cell above it carries sky 12, the
        // stone's own cell carries 0 (opaque cells hold no light).
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set_light(6, 7, 6, 12); // sky 12, block 0
        p.set_light(6, 6, 6, 0);
        let quads = mesh(&p);
        let top = quads.iter().find(|q| q.face == 2).unwrap();
        assert_eq!(top.skylight, 12, "top face lit by the cell above");
        assert_eq!(top.blocklight, 0);
    }

    #[test]
    fn torch_lit_out_cell_sets_blocklight() {
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set_light(7, 6, 6, 0x0E << 4); // sky 0, block 14 in front of +X
        let quads = mesh(&p);
        let px = quads.iter().find(|q| q.face == 0).unwrap();
        assert_eq!(px.blocklight, 14);
        assert_eq!(px.skylight, 0);
    }

    #[test]
    fn light_boundary_splits_the_greedy_merge() {
        // Two-block strip along x whose top out-cells differ: 15 vs 10.
        let mut p = PaddedSection::air();
        p.set(6, 6, 6, STONE);
        p.set(7, 6, 6, STONE);
        p.set_light(7, 7, 6, 10);
        let quads = mesh(&p);
        let tops: Vec<_> = quads.iter().filter(|q| q.face == 2 && q.y == 5).collect();
        assert_eq!(tops.len(), 2, "differing light must split the merge");
        let mut lights: Vec<u32> = tops.iter().map(|q| q.skylight).collect();
        lights.sort_unstable();
        assert_eq!(lights, vec![10, 15]);
    }

    #[test]
    fn uniform_light_still_merges_fully() {
        // Default uniform sky-15 everywhere: a full 32×32 slab's top face
        // must still merge to exactly one quad. The light key must not break
        // full-slab merging when light is uniform.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for z in 1..=32 {
                p.set(x, 1, z, GRASS);
            }
        }
        let quads = mesh(&p);
        assert_eq!(
            quads.iter().filter(|q| q.face == 2).count(),
            1,
            "uniform light must not split the slab merge"
        );
    }
}
