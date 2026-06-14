// Froxel volumetrics (M5c): god rays + height fog.
//
// A view-frustum-aligned 3D grid (VOL_W×VOL_H×VOL_D) covers VOL_NEAR..VOL_FAR
// with an exponential depth distribution (near slices dense). cs_inscatter
// writes per-froxel single-scatter radiance (rgb) + extinction (a); cs_integrate
// accumulates front-to-back into (accumulated in-scatter, transmittance). The
// composite pass samples the integrated grid and applies color·t + inscatter.
//
// Constants MUST mirror src/render/volumetric.rs.

struct VolUniform {
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>, // temporal reprojection (M5c Task 3)
    camera: vec4<f32>,           // xyz world pos, w frame index
    sun: vec4<f32>,              // xyz light dir (toward sun/moon), w 1=sun 0=moon
    sun_color: vec4<f32>,        // rgb light radiance
    sky: vec4<f32>,              // rgb ambient sky color (linear), w history valid
    fog: vec4<f32>,              // x density, y haze floor, z fog_y0, w fog_h
    tune: vec4<f32>,             // x absorb, y hg_g, z ambient, w taa_alpha
};

struct ShadowUniform {
    mats: array<mat4x4<f32>, 3>,
    splits: vec4<f32>, // cascade far view-distances
    texels: vec4<f32>, // world texel size per cascade
};

@group(0) @binding(0) var<uniform> vol: VolUniform;
@group(0) @binding(1) var<uniform> shadow: ShadowUniform;
@group(0) @binding(2) var shadow_map: texture_depth_2d_array;
@group(0) @binding(3) var shadow_samp: sampler_comparison;
@group(0) @binding(4) var in_scatter: texture_3d<f32>;   // cs_integrate reads this
@group(0) @binding(5) var prev_scatter: texture_3d<f32>; // cs_inscatter reprojects from this
@group(0) @binding(6) var lin_samp: sampler;             // bilinear, for prev_scatter

@group(1) @binding(0) var out_scatter: texture_storage_3d<rgba16float, write>;    // cs_inscatter writes
@group(1) @binding(1) var out_integrated: texture_storage_3d<rgba16float, write>; // cs_integrate writes

const VOL_W: u32 = 160u;
const VOL_H: u32 = 90u;
// Reduced from 64u to 48u (25% cut) to stay within GPU budget at 0.75× render-scale;
// per-pass tuning in M7 render-scale-default slice (Task 4C). Must mirror src/render/volumetric.rs.
const VOL_D: u32 = 48u;
const VOL_NEAR: f32 = 0.5;
const VOL_FAR: f32 = 360.0;
const PI: f32 = 3.14159265;

// Front edge of slice z (z may be fractional after jitter), in world meters.
fn slice_to_view_dist(z: f32) -> f32 {
    return VOL_NEAR * pow(VOL_FAR / VOL_NEAR, z / f32(VOL_D));
}

// Inverse: the (fractional) slice a view distance maps to. Mirrors volumetric.rs.
fn view_dist_to_slice(d: f32) -> f32 {
    return f32(VOL_D) * log(max(d, VOL_NEAR) / VOL_NEAR) / log(VOL_FAR / VOL_NEAR);
}

// Interleaved gradient noise (Jiménez 2014), animated per frame: a cheap
// per-froxel depth jitter so 1 sample/froxel doesn't band. TAA + the temporal
// reprojection denoise the residual.
fn ign(p: vec2<f32>, frame: f32) -> f32 {
    let q = p + 5.588238 * fract(frame * 0.61803398875);
    return fract(52.9829189 * fract(dot(q, vec2(0.06711056, 0.00583715))));
}

// Henyey-Greenstein phase: forward-scattering halo around the sun for g > 0.
fn hg_phase(cos_t: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = max(1.0 + g2 - 2.0 * g * cos_t, 1e-4);
    return (1.0 - g2) / (4.0 * PI * denom * sqrt(denom));
}

// Sun visibility from the CSM at a world point. Mirrors terrain.wgsl's cascade
// select, but with no normal-offset bias (froxels have no surface normal) and a
// single tap (the grid + temporal reproject supply the spatial smoothing).
fn shadow_vis(world_pos: vec3<f32>, view_dist: f32) -> f32 {
    var c: u32 = 3u;
    if view_dist < shadow.splits.x { c = 0u; }
    else if view_dist < shadow.splits.y { c = 1u; }
    else if view_dist < shadow.splits.z { c = 2u; }
    if c == 3u {
        return 1.0; // beyond the cascades: treat as lit (distant haze only)
    }
    let p = shadow.mats[c] * vec4(world_pos, 1.0);
    let uv = vec2(p.x, -p.y) * 0.5 + 0.5;
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 1.0;
    }
    return textureSampleCompareLevel(shadow_map, shadow_samp, uv, i32(c), p.z);
}

@compute @workgroup_size(4, 4, 4)
fn cs_inscatter(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= VOL_W || id.y >= VOL_H || id.z >= VOL_D { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2(f32(VOL_W), f32(VOL_H));
    let jitter = ign(vec2<f32>(id.xy), vol.camera.w);
    let view_dist = slice_to_view_dist(f32(id.z) + jitter);

    let ndc = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.5, 1.0);
    let world_h = vol.inv_view_proj * ndc;
    let ray = normalize(world_h.xyz / world_h.w - vol.camera.xyz);
    let world_pos = vol.camera.xyz + ray * view_dist;

    // Density pools fog in valleys; an exp height falloff above fog_y0 plus a
    // small uniform haze floor.
    let density = vol.fog.x * (vol.fog.y + exp(-max(0.0, world_pos.y - vol.fog.z) / vol.fog.w));
    let sigma_s = density;
    let sigma_e = max(density * (1.0 + vol.tune.x), 1e-5);

    let cos_t = dot(ray, vol.sun.xyz);
    let phase = hg_phase(cos_t, vol.tune.y);
    let vis = shadow_vis(world_pos, view_dist);
    let sun_in = vol.sun_color.rgb * vis * phase;
    let amb_in = vol.sky.rgb * vol.tune.z;
    var scatter = sigma_s * (sun_in + amb_in);

    // Temporal reprojection: blend against last frame's grid at this froxel's
    // world position (vol.sky.w = history valid; vol.tune.w = blend weight). The
    // single-sample-per-froxel estimate is noisy; reprojection + TAA denoise it.
    let alpha = select(1.0, vol.tune.w, vol.sky.w > 0.5);
    if alpha < 1.0 {
        let prev_clip = vol.prev_view_proj * vec4(world_pos, 1.0);
        if prev_clip.w > 0.0 {
            let prev_ndc = prev_clip.xyz / prev_clip.w;
            let prev_uv = vec2(prev_ndc.x * 0.5 + 0.5, 0.5 - prev_ndc.y * 0.5);
            // Camera barely moves per frame, so reuse this froxel's view distance
            // for the depth slice; the XY reprojection captures the real motion.
            let prev_w = clamp(view_dist_to_slice(view_dist) / f32(VOL_D), 0.0, 1.0);
            let inside = all(prev_uv >= vec2(0.0)) && all(prev_uv <= vec2(1.0));
            if inside {
                let hist = textureSampleLevel(prev_scatter, lin_samp, vec3(prev_uv, prev_w), 0.0);
                scatter = mix(hist.rgb, scatter, alpha);
            }
        }
    }

    textureStore(out_scatter, vec3<i32>(id), vec4(scatter, sigma_e));
}

@compute @workgroup_size(8, 8, 1)
fn cs_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= VOL_W || id.y >= VOL_H { return; }
    var accum = vec3(0.0);
    var trans = 1.0;
    var prev_d = VOL_NEAR;
    for (var z: u32 = 0u; z < VOL_D; z++) {
        let coord = vec3<i32>(i32(id.x), i32(id.y), i32(z));
        let s = textureLoad(in_scatter, coord, 0);
        let d = slice_to_view_dist(f32(z) + 1.0);
        let dt = max(d - prev_d, 0.0);
        prev_d = d;
        let sigma_e = max(s.a, 1e-5);
        let t_slice = exp(-sigma_e * dt);
        // Energy-conserving slice integral (Frostbite 2015): integrate the
        // in-scatter against transmittance over the slice rather than point-sampling.
        let s_int = (s.rgb - s.rgb * t_slice) / sigma_e;
        accum += trans * s_int;
        trans *= t_slice;
        textureStore(out_integrated, coord, vec4(accum, trans));
    }
}
