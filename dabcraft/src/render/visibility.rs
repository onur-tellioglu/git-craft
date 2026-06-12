// Cave culling (spec §6, Tommaso Checchi): per-section face-to-face
// visibility masks computed at mesh time, consumed by a per-frame BFS
// from the camera section (Task 11).

use std::collections::{HashSet, VecDeque};

use crate::mesh::padded::PaddedSection;
use crate::world::chunks::SectionPos;

/// Face index → step direction, mesher face order.
const FACE_STEP: [(i32, i32, i32); 6] =
    [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];
const OPPOSITE: [usize; 6] = [1, 0, 3, 2, 5, 4];

/// Per-frame cave-culling BFS (spec §6). Walk outward from the camera's
/// section; enter a neighbor only through faces the current section's mask
/// connects to the face we entered through; never step opposite to a
/// direction already traveled on this path (anti-wraparound); visited is
/// keyed by position (first arrival wins — vanilla's simplification,
/// conservative in pathological spirals). `mask_of` returning `None`
/// (section not meshed yet) traverses freely: unknown must never hide
/// geometry. Result: every section that may be visible; the frustum test
/// in `TerrainRenderer::prepare` intersects it.
pub fn visible_set(
    camera: SectionPos,
    radius: i32,
    mask_of: impl Fn(SectionPos) -> Option<u16>,
) -> HashSet<SectionPos> {
    let start = SectionPos { x: camera.x, y: camera.y.clamp(0, 7), z: camera.z };
    let r2 = radius * radius;
    let mut visited: HashSet<SectionPos> = HashSet::new();
    // (section, face we entered through, directions traveled so far)
    let mut queue: VecDeque<(SectionPos, Option<usize>, u8)> = VecDeque::new();
    visited.insert(start);
    queue.push_back((start, None, 0));
    while let Some((pos, entry, dirs)) = queue.pop_front() {
        for f in 0..6usize {
            if dirs & (1 << OPPOSITE[f]) != 0 {
                continue; // would step back toward the camera
            }
            if let Some(e) = entry {
                // Entering through e and leaving through f needs the pair
                // connected. f == e is unreachable: entering through e means
                // we traveled OPPOSITE(e), so the dirs check above bars f == e.
                if let Some(m) = mask_of(pos)
                    && m & pair_bit(e, f) == 0
                {
                    continue;
                }
            }
            let (dx, dy, dz) = FACE_STEP[f];
            let n = SectionPos { x: pos.x + dx, y: pos.y + dy, z: pos.z + dz };
            if !(0..8).contains(&n.y) {
                continue;
            }
            if (n.x - start.x).pow(2) + (n.z - start.z).pow(2) > r2 {
                continue;
            }
            if !visited.insert(n) {
                continue;
            }
            queue.push_back((n, Some(OPPOSITE[f]), dirs | (1 << f)));
        }
    }
    visited
}

/// Bit index for the unordered face pair (a, b), a ≠ b, faces in mesher
/// order 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z. 15 pairs → bits 0..15.
pub fn pair_bit(a: usize, b: usize) -> u16 {
    debug_assert!(a < 6 && b < 6 && a != b);
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    // Pairs ordered (0,1)(0,2)..(0,5)(1,2)..(4,5): offset of row `lo` is
    // lo*5 - lo*(lo-1)/2 in triangular numbering.
    const ROW_OFFSET: [u16; 5] = [0, 5, 9, 12, 14];
    1 << (ROW_OFFSET[lo] + (hi - lo - 1) as u16)
}

const SIZE: usize = 32;

fn vidx(x: usize, y: usize, z: usize) -> usize {
    (y * SIZE + z) * SIZE + x
}

/// Flood-fill the section's non-solid interior voxels into components; OR
/// the face-pair bits of every pair of faces a component touches. Costs
/// one pass over 32³ at mesh time (runs in the mesh job).
pub fn face_connectivity(padded: &PaddedSection) -> u16 {
    let mut visited = [0u64; SIZE * SIZE * SIZE / 64];
    let mut mask = 0u16;
    let mut stack: Vec<(usize, usize, usize)> = Vec::new();
    for sy in 0..SIZE {
        for sz in 0..SIZE {
            for sx in 0..SIZE {
                let i = vidx(sx, sy, sz);
                if visited[i / 64] & (1 << (i % 64)) != 0 {
                    continue;
                }
                // Padded offset +1: interior voxel (sx,sy,sz).
                if padded.get(sx + 1, sy + 1, sz + 1).is_solid() {
                    continue;
                }
                // New component: flood it, recording touched faces.
                let mut faces = 0u8;
                visited[i / 64] |= 1 << (i % 64);
                stack.push((sx, sy, sz));
                while let Some((x, y, z)) = stack.pop() {
                    if x == SIZE - 1 { faces |= 1; }      // +X
                    if x == 0        { faces |= 1 << 1; } // -X
                    if y == SIZE - 1 { faces |= 1 << 2; } // +Y
                    if y == 0        { faces |= 1 << 3; } // -Y
                    if z == SIZE - 1 { faces |= 1 << 4; } // +Z
                    if z == 0        { faces |= 1 << 5; } // -Z
                    for (dx, dy, dz) in
                        [(1i32, 0i32, 0i32), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)]
                    {
                        let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
                        if !(0..SIZE as i32).contains(&nx)
                            || !(0..SIZE as i32).contains(&ny)
                            || !(0..SIZE as i32).contains(&nz)
                        {
                            continue;
                        }
                        let (nx, ny, nz) = (nx as usize, ny as usize, nz as usize);
                        let ni = vidx(nx, ny, nz);
                        if visited[ni / 64] & (1 << (ni % 64)) != 0 {
                            continue;
                        }
                        if padded.get(nx + 1, ny + 1, nz + 1).is_solid() {
                            continue;
                        }
                        visited[ni / 64] |= 1 << (ni % 64);
                        stack.push((nx, ny, nz));
                    }
                }
                for a in 0..6usize {
                    for b in (a + 1)..6usize {
                        if faces & (1 << a) != 0 && faces & (1 << b) != 0 {
                            mask |= pair_bit(a, b);
                        }
                    }
                }
            }
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::padded::PaddedSection;
    use crate::world::block::STONE;

    const ALL_PAIRS: u16 = 0x7FFF;

    #[test]
    fn pair_bits_are_distinct_and_cover_15() {
        let mut seen = 0u16;
        for a in 0..6 {
            for b in 0..6 {
                if a == b {
                    continue;
                }
                let bit = pair_bit(a, b);
                assert_eq!(bit.count_ones(), 1);
                assert_eq!(bit, pair_bit(b, a), "order-free");
                seen |= bit;
            }
        }
        assert_eq!(seen, ALL_PAIRS);
    }

    #[test]
    fn empty_section_connects_all_faces() {
        assert_eq!(face_connectivity(&PaddedSection::air()), ALL_PAIRS);
    }

    #[test]
    fn solid_section_connects_nothing() {
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    p.set(x, y, z, STONE);
                }
            }
        }
        assert_eq!(face_connectivity(&p), 0);
    }

    #[test]
    fn straight_tunnel_connects_only_its_two_faces() {
        // Solid section with a 1×1 tunnel along X at y=16, z=16.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    if y != 17 || z != 17 {
                        p.set(x, y, z, STONE);
                    }
                }
            }
        }
        // Faces: 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z (mesher order).
        assert_eq!(face_connectivity(&p), pair_bit(0, 1));
    }

    #[test]
    fn l_tunnel_connects_the_corner_pair() {
        // Tunnel entering -X at y=17,z=17 to the center, then turning +Z.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    let leg_x = y == 17 && z == 17 && x <= 16;
                    let leg_z = y == 17 && x == 16 && z >= 17;
                    if !(leg_x || leg_z) {
                        p.set(x, y, z, STONE);
                    }
                }
            }
        }
        assert_eq!(face_connectivity(&p), pair_bit(1, 4), "-X to +Z only");
    }

    #[test]
    fn disjoint_components_union_their_pairs() {
        // Two parallel tunnels: one along X (y=5,z=5), one along Z (y=20,x=20).
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    let tx = y == 6 && z == 6;
                    let tz = y == 21 && x == 21;
                    if !(tx || tz) {
                        p.set(x, y, z, STONE);
                    }
                }
            }
        }
        assert_eq!(face_connectivity(&p), pair_bit(0, 1) | pair_bit(4, 5));
    }

    #[test]
    fn dead_end_pocket_adds_no_pairs() {
        // A pocket touching only the -X face: you can see in, not through.
        let mut p = PaddedSection::air();
        for x in 1..=32 {
            for y in 1..=32 {
                for z in 1..=32 {
                    let pocket = x <= 4 && y == 17 && z == 17;
                    if !pocket {
                        p.set(x, y, z, STONE);
                    }
                }
            }
        }
        assert_eq!(face_connectivity(&p), 0, "single-face component yields no pair");
    }

    use crate::world::chunks::SectionPos;
    use std::collections::HashMap;

    fn sp(x: i32, y: i32, z: i32) -> SectionPos {
        SectionPos { x, y, z }
    }

    #[test]
    fn open_world_reaches_the_whole_radius() {
        // No masks known at all: everything within radius and 0..8 vertically.
        let vis = visible_set(sp(0, 4, 0), 3, |_| None);
        assert!(vis.contains(&sp(0, 4, 0)));
        assert!(vis.contains(&sp(3, 4, 0)), "cardinal edge of the radius");
        assert!(vis.contains(&sp(2, 7, 2)));
        assert!(!vis.contains(&sp(4, 4, 0)), "outside the radius");
        assert!(!vis.contains(&sp(0, 8, 0)), "above the world");
    }

    #[test]
    fn fully_open_masks_match_unknown_masks() {
        let all = visible_set(sp(0, 4, 0), 2, |_| Some(0x7FFF));
        let unknown = visible_set(sp(0, 4, 0), 2, |_| None);
        assert_eq!(all, unknown);
    }

    #[test]
    fn sealed_sections_stop_the_walk() {
        // Everything at x >= 1 is sealed (mask 0): the BFS enters the x=1
        // wall sections (drawn — you see their face) but never passes them.
        let vis = visible_set(sp(0, 4, 0), 3, |p| Some(if p.x >= 1 { 0 } else { 0x7FFF }));
        assert!(vis.contains(&sp(1, 4, 0)), "the wall itself is visible");
        assert!(!vis.contains(&sp(2, 4, 0)), "nothing behind the wall");
        assert!(vis.contains(&sp(-3, 4, 0)), "open side unaffected");
    }

    #[test]
    fn tunnel_masks_gate_by_entry_face() {
        // Section (1,4,0) connects only -X↔+X; camera at origin:
        let masks: HashMap<SectionPos, u16> = HashMap::from([
            (sp(1, 4, 0), pair_bit(0, 1)),
        ]);
        let default_open = 0x7FFF;
        let vis = visible_set(sp(0, 4, 0), 3, |p| Some(*masks.get(&p).unwrap_or(&default_open)));
        assert!(vis.contains(&sp(2, 4, 0)), "straight through the X tunnel");
        // (1,4,1) is reachable from (0,4,0) via (0,4,1) — open sections —
        // but NOT through the tunnel section's +Z face:
        assert!(vis.contains(&sp(1, 4, 1)), "reached around, not through");
        // Wall off the around-path to prove the tunnel doesn't leak +Z:
        let masks2: HashMap<SectionPos, u16> = HashMap::from([
            (sp(1, 4, 0), pair_bit(0, 1)),
            (sp(0, 4, 1), 0u16),
            (sp(0, 4, -1), 0u16),
            (sp(0, 3, 0), 0u16),
            (sp(0, 5, 0), 0u16),
            (sp(-1, 4, 0), 0u16),
        ]);
        let vis2 = visible_set(sp(0, 4, 0), 3, |p| Some(*masks2.get(&p).unwrap_or(&default_open)));
        assert!(vis2.contains(&sp(2, 4, 0)), "tunnel pass-through still works");
        assert!(!vis2.contains(&sp(1, 4, 1)), "tunnel's +Z face is not connected to its -X entry");
    }

    #[test]
    fn camera_above_the_world_clamps_and_sees_down() {
        let vis = visible_set(sp(0, 12, 0), 2, |_| None);
        assert!(vis.contains(&sp(0, 7, 0)), "clamped start at the world top");
        assert!(vis.contains(&sp(1, 6, 1)));
    }
}
