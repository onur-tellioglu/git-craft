use glam::IVec3;

use crate::world::block::{CACTUS, OAK_LEAVES, OAK_LOG, SPRUCE_LEAVES, SPRUCE_LOG};
use crate::world::r#gen::StructureWrite;

/// splitmix64 over (seed, x, z, salt): deterministic decoration decisions.
pub fn hash_pos(seed: i32, x: i32, z: i32, salt: u64) -> u64 {
    let mut v = (seed as u64)
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    v ^= v >> 30;
    v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    v ^= v >> 27;
    v = v.wrapping_mul(0x94D0_49BB_1331_11EB);
    v ^= v >> 31;
    v
}

/// Oak: straight trunk, 5×5 canopy (minus corners) on the top two trunk
/// levels, 3×3 cross above. `base` is the ground-surface block; the trunk
/// starts one above it.
pub fn oak_tree(base: IVec3, trunk: i32) -> Vec<StructureWrite> {
    let mut w = Vec::with_capacity(64);
    let top = base.y + trunk;
    for y in (base.y + 1)..=top {
        w.push(StructureWrite {
            pos: IVec3::new(base.x, y, base.z),
            block: OAK_LOG,
            only_air: false,
        });
    }
    let mut leaf = |x: i32, y: i32, z: i32| {
        w.push(StructureWrite {
            pos: IVec3::new(x, y, z),
            block: OAK_LEAVES,
            only_air: true,
        });
    };
    for y in (top - 1)..=top {
        for dx in -2i32..=2 {
            for dz in -2i32..=2 {
                if dx == 0 && dz == 0 {
                    continue; // trunk occupies the center
                }
                if dx.abs() == 2 && dz.abs() == 2 {
                    continue; // clip corners
                }
                leaf(base.x + dx, y, base.z + dz);
            }
        }
    }
    for dx in -1i32..=1 {
        for dz in -1i32..=1 {
            if dx.abs() == 1 && dz.abs() == 1 {
                continue;
            }
            leaf(base.x + dx, top + 1, base.z + dz);
        }
    }
    w
}

/// Spruce: narrow — 3×3-minus-corners ring at the top, full 3×3 rings every
/// two levels below, single cap leaf two above the top log.
pub fn spruce_tree(base: IVec3, trunk: i32) -> Vec<StructureWrite> {
    let mut w = Vec::with_capacity(32);
    let top = base.y + trunk;
    for y in (base.y + 1)..=top {
        w.push(StructureWrite {
            pos: IVec3::new(base.x, y, base.z),
            block: SPRUCE_LOG,
            only_air: false,
        });
    }
    let mut leaf = |x: i32, y: i32, z: i32| {
        w.push(StructureWrite {
            pos: IVec3::new(x, y, z),
            block: SPRUCE_LEAVES,
            only_air: true,
        });
    };
    for ring in 0..3 {
        let y = top - ring * 2 + 1;
        if y <= base.y + 2 {
            break;
        }
        for dx in -1i32..=1 {
            for dz in -1i32..=1 {
                if dx == 0 && dz == 0 {
                    continue;
                }
                if dx.abs() == 1 && dz.abs() == 1 && ring == 0 {
                    continue; // pointy top ring
                }
                leaf(base.x + dx, y, base.z + dz);
            }
        }
    }
    leaf(base.x, top + 2, base.z);
    w
}

pub fn cactus_plant(base: IVec3, height: i32) -> Vec<StructureWrite> {
    (1..=height)
        .map(|dy| StructureWrite {
            pos: IVec3::new(base.x, base.y + dy, base.z),
            block: CACTUS,
            only_air: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{CACTUS, OAK_LOG, SPRUCE_LOG};
    use crate::world::r#gen::{Biome, WorldGen};

    #[test]
    fn hash_is_deterministic_and_position_sensitive() {
        assert_eq!(hash_pos(7, 100, -5, 1), hash_pos(7, 100, -5, 1));
        assert_ne!(hash_pos(7, 100, -5, 1), hash_pos(7, 101, -5, 1));
        assert_ne!(hash_pos(7, 100, -5, 1), hash_pos(8, 100, -5, 1));
        assert_ne!(hash_pos(7, 100, -5, 1), hash_pos(7, 100, -5, 2));
    }

    #[test]
    fn oak_tree_shape_is_well_formed() {
        let writes = oak_tree(glam::IVec3::new(10, 70, 10), 5);
        let logs: Vec<_> = writes.iter().filter(|w| w.block == OAK_LOG).collect();
        let leaves: Vec<_> = writes
            .iter()
            .filter(|w| w.block == crate::world::block::OAK_LEAVES)
            .collect();
        assert_eq!(logs.len(), 5, "trunk height");
        assert!(logs.iter().all(|w| !w.only_air), "logs overwrite");
        assert!(leaves.iter().all(|w| w.only_air), "leaves never overwrite");
        assert!(leaves.len() > 20, "canopy exists");
        let top_log = logs.iter().map(|w| w.pos.y).max().unwrap();
        assert!(
            leaves.iter().any(|w| w.pos.y > top_log),
            "leaves above the trunk"
        );
    }

    #[test]
    fn forest_columns_contain_trees() {
        // Some forest column within ±48 columns must contain an oak log.
        let wg = WorldGen::new(1337);
        let any_tree = (-48..48).any(|cx| {
            (-48..48).any(|cz| {
                wg.biome(cx * 32 + 16, cz * 32 + 16) == Biome::Forest && {
                    let (col, _) = wg.generate_column(cx, cz);
                    col.sections.iter().any(|s| {
                        let mut flat = vec![crate::world::block::AIR; 32 * 32 * 32];
                        s.unpack_into(&mut flat);
                        flat.contains(&OAK_LOG)
                    })
                }
            })
        });
        assert!(any_tree, "no trees in any forest column in a 96×96 area");
    }

    #[test]
    fn border_trees_emit_out_of_column_writes() {
        // Canopies are 5 wide: trunks near a column edge must spill writes.
        // Find one spilling column and verify all returned writes are outside.
        let wg = WorldGen::new(1337);
        for cx in -48..48 {
            for cz in -48..48 {
                let (_, writes) = wg.generate_column(cx, cz);
                if writes.is_empty() {
                    continue;
                }
                for w in &writes {
                    let in_x = (cx * 32..(cx + 1) * 32).contains(&w.pos.x);
                    let in_z = (cz * 32..(cz + 1) * 32).contains(&w.pos.z);
                    assert!(
                        !(in_x && in_z),
                        "returned write {:?} is inside its own column",
                        w.pos
                    );
                }
                return; // one spilling column is proof enough
            }
        }
        panic!("no border-crossing structure found in 96×96 columns");
    }

    #[test]
    fn spruce_and_cactus_shapes() {
        let spruce = spruce_tree(glam::IVec3::new(0, 100, 0), 6);
        assert_eq!(spruce.iter().filter(|w| w.block == SPRUCE_LOG).count(), 6);
        let cactus = cactus_plant(glam::IVec3::new(0, 65, 0), 2);
        assert_eq!(cactus.len(), 2);
        assert!(cactus.iter().all(|w| w.block == CACTUS));
    }
}
