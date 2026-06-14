//! Procedurally-generated block material textures (albedo + roughness + normal).
//!
//! The project ships no proprietary art, so each block's material is generated
//! in code from a deterministic hash, seeded by the block's own base color
//! (`block.color()`). The result is two `Rgba8Unorm` texture-array payloads —
//! albedo (RGB) + roughness (A), and a tangent-space normal map (RGB) — each
//! with a full CPU-built mip chain so terrain tiling doesn't shimmer at range.
//! Everything here is pure; `terrain.rs` uploads it to the GPU.

use crate::world::block::BlockId;

/// One texel axis of each block's material tile.
pub const ATLAS_SIZE: u32 = 16;
/// Layers = block ids `0..=12` (air at 0 is generated but never rendered).
pub const ATLAS_LAYERS: u32 = 13;

/// Two texture-array payloads (albedo+roughness, normal), each as a mip chain.
pub struct MaterialAtlas {
    pub size: u32,
    pub layers: u32,
    pub mip_levels: u32,
    /// Per mip level: `layers × w × h × 4` bytes, layer-major. RGB = linear
    /// albedo, A = roughness.
    pub albedo: Vec<Vec<u8>>,
    /// Per mip level: `layers × w × h × 4` bytes, layer-major. RGB = encoded
    /// tangent-space normal (`n*0.5+0.5`), A unused.
    pub normal: Vec<Vec<u8>>,
}

/// Integer bit-mix hash (deterministic, no RNG state).
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// Hash a lattice point to `0..1`.
fn rand01(gx: u32, gy: u32, layer: u32, salt: u32) -> f32 {
    let h = hash(
        gx.wrapping_mul(374_761_393)
            ^ gy.wrapping_mul(668_265_263)
            ^ layer.wrapping_mul(2_246_822_519)
            ^ salt.wrapping_mul(3_266_489_917),
    );
    (h >> 8) as f32 / (1u32 << 24) as f32
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Tiling value noise: bilinear over a `freq×freq` lattice that wraps, so the
/// texture is seamless under a repeat sampler. Returns `0..1`.
fn value_noise(x: u32, y: u32, size: u32, layer: u32, freq: u32, salt: u32) -> f32 {
    let fx = x as f32 / size as f32 * freq as f32;
    let fy = y as f32 / size as f32 * freq as f32;
    let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let g = |gx: u32, gy: u32| rand01(gx % freq, gy % freq, layer, salt);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let top = lerp(g(x0, y0), g(x0 + 1, y0), sx);
    let bot = lerp(g(x0, y0 + 1), g(x0 + 1, y0 + 1), sx);
    lerp(top, bot, sy)
}

/// Per-block surface roughness (0 = mirror, 1 = fully matte).
fn block_roughness(id: u32) -> f32 {
    match id {
        5 => 0.7,     // snow grass: smoother
        6 => 0.4,     // water (own pass, but kept consistent)
        7 | 9 => 0.8, // logs
        12 => 0.6,    // torch
        _ => 0.92,    // grass/dirt/stone/sand/leaves/cactus: matte
    }
}

/// Per-block bump strength for the normal map (rougher rock = more relief).
fn block_bump(id: u32) -> f32 {
    match id {
        3 => 1.6,      // stone
        2 | 4 => 1.2,  // dirt, sand
        5 => 0.4,      // snow: nearly flat
        8 | 10 => 1.4, // leaves: dappled
        _ => 1.0,
    }
}

fn build_layer(layer: u32, size: u32, albedo: &mut [u8], normal: &mut [u8]) {
    let base = BlockId(layer as u16).color();
    let rough = (block_roughness(layer) * 255.0).round() as u8;
    let bump = block_bump(layer);

    // Height field for the normal map; two octaves of tiling value noise.
    let height = |x: u32, y: u32| {
        let a = value_noise(x, y, size, layer, 8, 1);
        let b = value_noise(x, y, size, layer, 16, 2);
        a * 0.65 + b * 0.35
    };

    for y in 0..size {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;

            // Albedo: base color modulated by gentle two-octave detail (±~12%).
            let n = value_noise(x, y, size, layer, 8, 3) * 0.6
                + value_noise(x, y, size, layer, 16, 4) * 0.4;
            let detail = 1.0 + (n - 0.5) * 0.25;
            for c in 0..3 {
                albedo[i + c] = ((base[c] * detail).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            albedo[i + 3] = rough;

            // Tangent-space normal from central differences of the height field
            // (wrapping so the tile stays seamless).
            let xl = (x + size - 1) % size;
            let xr = (x + 1) % size;
            let yl = (y + size - 1) % size;
            let yr = (y + 1) % size;
            let dx = (height(xr, y) - height(xl, y)) * bump;
            let dy = (height(x, yr) - height(x, yl)) * bump;
            let inv = 1.0 / (dx * dx + dy * dy + 1.0).sqrt();
            let (nx, ny, nz) = (-dx * inv, -dy * inv, inv);
            normal[i] = ((nx * 0.5 + 0.5) * 255.0).round() as u8;
            normal[i + 1] = ((ny * 0.5 + 0.5) * 255.0).round() as u8;
            normal[i + 2] = ((nz * 0.5 + 0.5) * 255.0).round() as u8;
            normal[i + 3] = 255;
        }
    }
}

/// Box-filter `src` (layers × w × h × 4, layer-major) to half resolution.
fn downsample(src: &[u8], layers: u32, w: u32, h: u32) -> Vec<u8> {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut dst = vec![0u8; (layers * nw * nh * 4) as usize];
    for layer in 0..layers {
        let so = (layer * w * h * 4) as usize;
        let dst_o = (layer * nw * nh * 4) as usize;
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let sample = |sx: u32, sy: u32| {
                        src[so + ((sy.min(h - 1) * w + sx.min(w - 1)) * 4) as usize + c] as u32
                    };
                    let avg = (sample(x * 2, y * 2)
                        + sample(x * 2 + 1, y * 2)
                        + sample(x * 2, y * 2 + 1)
                        + sample(x * 2 + 1, y * 2 + 1))
                        / 4;
                    dst[dst_o + ((y * nw + x) * 4) as usize + c] = avg as u8;
                }
            }
        }
    }
    dst
}

/// Generate the full material atlas at `size × size` per layer.
pub fn build_atlas(size: u32) -> MaterialAtlas {
    let layers = ATLAS_LAYERS;
    let mut albedo0 = vec![0u8; (layers * size * size * 4) as usize];
    let mut normal0 = vec![0u8; (layers * size * size * 4) as usize];
    for layer in 0..layers {
        let o = (layer * size * size * 4) as usize;
        let end = o + (size * size * 4) as usize;
        build_layer(layer, size, &mut albedo0[o..end], &mut normal0[o..end]);
    }

    let mip_levels = size.ilog2() + 1;
    let mut albedo = vec![albedo0];
    let mut normal = vec![normal0];
    let mut w = size;
    for _ in 1..mip_levels {
        let prev_a = albedo.last().unwrap();
        let prev_n = normal.last().unwrap();
        albedo.push(downsample(prev_a, layers, w, w));
        normal.push(downsample(prev_n, layers, w, w));
        w = (w / 2).max(1);
    }

    MaterialAtlas {
        size,
        layers,
        mip_levels,
        albedo,
        normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_is_deterministic() {
        let a = build_atlas(ATLAS_SIZE);
        let b = build_atlas(ATLAS_SIZE);
        assert_eq!(a.albedo, b.albedo);
        assert_eq!(a.normal, b.normal);
    }

    #[test]
    fn dimensions_and_mip_count() {
        let atlas = build_atlas(ATLAS_SIZE);
        assert_eq!(atlas.size, ATLAS_SIZE);
        assert_eq!(atlas.layers, ATLAS_LAYERS);
        assert_eq!(atlas.mip_levels, 5); // 16,8,4,2,1
        assert_eq!(atlas.albedo.len(), 5);
        let mut w = ATLAS_SIZE;
        for mip in 0..atlas.mip_levels as usize {
            let expected = (ATLAS_LAYERS * w * w * 4) as usize;
            assert_eq!(atlas.albedo[mip].len(), expected, "albedo mip {mip}");
            assert_eq!(atlas.normal[mip].len(), expected, "normal mip {mip}");
            w = (w / 2).max(1);
        }
    }

    #[test]
    fn albedo_mean_tracks_block_color() {
        // block.color() is the authoritative palette: the detail noise is a
        // symmetric ±12% around the base, so a layer's mean albedo ≈ the base.
        use crate::world::block::{GRASS, STONE};
        let atlas = build_atlas(ATLAS_SIZE);
        let n = (ATLAS_SIZE * ATLAS_SIZE) as usize;
        let mean = |id: u32| -> [f32; 3] {
            let o = (id * ATLAS_SIZE * ATLAS_SIZE * 4) as usize;
            let mut sum = [0.0f32; 3];
            for px in atlas.albedo[0][o..o + n * 4].chunks_exact(4) {
                for c in 0..3 {
                    sum[c] += px[c] as f32 / 255.0;
                }
            }
            sum.map(|s| s / n as f32)
        };
        for id in [STONE, GRASS] {
            let base = id.color();
            let m = mean(id.0 as u32);
            for c in 0..3 {
                assert!(
                    (m[c] - base[c]).abs() < 0.05,
                    "block {} channel {c}: mean {} vs base {}",
                    id.0,
                    m[c],
                    base[c]
                );
            }
        }
    }

    #[test]
    fn different_blocks_have_different_albedo() {
        let atlas = build_atlas(ATLAS_SIZE);
        let layer = |id: u32| {
            let o = (id * ATLAS_SIZE * ATLAS_SIZE * 4) as usize;
            atlas.albedo[0][o..o + (ATLAS_SIZE * ATLAS_SIZE * 4) as usize].to_vec()
        };
        assert_ne!(layer(1), layer(3), "grass vs stone must differ");
        assert_ne!(layer(2), layer(4), "dirt vs sand must differ");
    }

    #[test]
    fn normals_point_outward_at_mip0() {
        let atlas = build_atlas(ATLAS_SIZE);
        // The encoded z (blue) channel must stay >= 128 (n.z >= 0) everywhere.
        for (i, px) in atlas.normal[0].chunks_exact(4).enumerate() {
            assert!(px[2] >= 128, "texel {i} normal points inward: z={}", px[2]);
        }
    }

    #[test]
    fn roughness_is_in_range_and_per_block() {
        let atlas = build_atlas(ATLAS_SIZE);
        let alpha = |id: u32| {
            let o = (id * ATLAS_SIZE * ATLAS_SIZE * 4) as usize;
            atlas.albedo[0][o + 3] // first texel's alpha (roughness is per-block uniform)
        };
        // Snow (5) is smoother than stone (3).
        assert!(alpha(5) < alpha(3));
        for id in 0..ATLAS_LAYERS {
            assert!(alpha(id) > 0, "roughness must be non-zero for block {id}");
        }
    }

    #[test]
    fn a_flat_layer_stays_uniform_through_mips() {
        // A constant-color buffer must box-filter to the same constant.
        let layers = 1;
        let mut buf = vec![0u8; (4 * 4 * 4) as usize];
        for px in buf.chunks_exact_mut(4) {
            px.copy_from_slice(&[40, 80, 120, 200]);
        }
        let half = downsample(&buf, layers, 4, 4);
        assert!(half.chunks_exact(4).all(|px| px == [40, 80, 120, 200]));
        assert_eq!(half.len(), (4 * 2 * 2) as usize);
    }
}
