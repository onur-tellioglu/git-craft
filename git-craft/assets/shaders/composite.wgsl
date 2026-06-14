// Composite pass: apply GTAO to the ambient term, then volumetric fog/god rays.
//
// AO:   factor = 1 - ambientWeight*(1 - ao); ambientWeight (g-buffer alpha) is
//       the sky-ambient fraction, so direct sun and torch light stay un-darkened.
// Vol:  reconstruct view distance from depth, sample the integrated froxel grid,
//       and apply color·transmittance + in-scatter (god rays + height fog).
// TAA reads the composited result, so it stabilizes residual AO/fog noise.

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var gbuf_tex: texture_2d<f32>;
@group(0) @binding(2) var ao_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct CompUniform {
    flags: vec4<f32>,           // x: AO debug, y: volumetric in-scatter debug
    inv_view_proj: mat4x4<f32>, // jittered (matches the depth buffer)
    camera: vec4<f32>,          // xyz world pos
    vol_params: vec4<f32>,      // x VOL_NEAR, y VOL_FAR, z VOL_D
}
@group(0) @binding(4) var<uniform> c: CompUniform;
@group(0) @binding(5) var froxel_tex: texture_3d<f32>;     // integrated grid (rgb in-scatter, a transmittance)
@group(0) @binding(6) var depth_tex: texture_2d<f32>;       // scene depth (DepthOnly view bound as float)

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

// Inverse of volumetric.wgsl's slice_to_view_dist; mirrors volumetric.rs.
fn view_dist_to_slice(d: f32) -> f32 {
    let near = c.vol_params.x;
    let far = c.vol_params.y;
    return c.vol_params.z * log(max(d, near) / near) / log(far / near);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.pos.xy);
    let hdr = textureLoad(hdr_tex, px, 0).rgb;
    let ambient_weight = textureLoad(gbuf_tex, px, 0).a;
    // Half-res AO, bilinearly upsampled.
    let ao = textureSampleLevel(ao_tex, samp, in.uv, 0.0).r;
    let factor = 1.0 - ambient_weight * (1.0 - ao);
    var color = hdr * factor;

    // Volumetric: pick the froxel slice from the per-pixel view distance. Sky
    // pixels (depth == 1) sample the far slice so distant haze still applies.
    let depth = textureLoad(depth_tex, px, 0).r;
    var view_dist = c.vol_params.y;
    if depth < 1.0 {
        let world_h = c.inv_view_proj * vec4(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, depth, 1.0);
        view_dist = length(world_h.xyz / world_h.w - c.camera.xyz);
    }
    let w = clamp(view_dist_to_slice(view_dist) / c.vol_params.z, 0.0, 1.0);
    let fog = textureSampleLevel(froxel_tex, samp, vec3(in.uv, w), 0.0);
    color = color * fog.a + fog.rgb;

    if c.flags.x > 0.5 {
        return vec4(vec3(ao), 1.0); // AO debug view
    }
    if c.flags.y > 0.5 {
        return vec4(fog.rgb, 1.0); // volumetric in-scatter debug view
    }
    return vec4(color, 1.0);
}
