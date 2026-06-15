// Temporal AA resolve — reprojects depth to world space, finds the history
// texel via the previous frame's unjittered VP, clamps history into the
// current 5-tap plus-cross neighborhood (kills ghosting), and blends with a
// disocclusion guard that leans on the current sample where history is stale.

struct TaaUniform {
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    params: vec4<f32>, // xy viewport px, z blend, w valid
};

@group(0) @binding(0) var current_tex: texture_2d<f32>;
@group(0) @binding(1) var history_tex: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var<uniform> u: TaaUniform;

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.pos.xy);
    let current = textureLoad(current_tex, px, 0).rgb;

    // Reconstruct world position from this frame's (jittered) depth.
    let depth = textureLoad(depth_tex, px, 0).r;
    let ndc = vec3(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, depth);
    let world_h = u.inv_view_proj * vec4(ndc, 1.0);
    let world = world_h.xyz / world_h.w;

    // Reproject to last frame's screen (unjittered) to find the history texel.
    let prev_clip = u.prev_view_proj * vec4(world, 1.0);
    let prev_ndc = prev_clip.xyz / prev_clip.w;
    let prev_uv = vec2(prev_ndc.x * 0.5 + 0.5, -prev_ndc.y * 0.5 + 0.5);

    let on_screen = all(prev_uv >= vec2(0.0)) && all(prev_uv <= vec2(1.0));
    if u.params.w < 0.5 || !on_screen || depth >= 1.0 {
        // First frame / disoccluded / sky pixel: take current, no history.
        return vec4(current, 1.0);
    }

    var history = textureSampleLevel(history_tex, samp, prev_uv, 0.0).rgb;

    // Neighborhood AABB clamp (kills ghosting): bound history to the colour
    // box of a 5-tap plus-shaped cross (center + 4 cardinal neighbors).
    // Drops the 4 diagonal corners vs. the old 3×3 box (~44% fewer loads).
    // The AABB is slightly looser on diagonals; the ghost-blend below
    // compensates by leaning on current when history diverges.
    var lo = current;
    var hi = current;
    let neighbors = array<vec2<i32>, 4>(
        vec2( 0, -1),
        vec2(-1,  0),
        vec2( 1,  0),
        vec2( 0,  1),
    );
    for (var i = 0; i < 4; i++) {
        let c = textureLoad(current_tex, px + neighbors[i], 0).rgb;
        lo = min(lo, c);
        hi = max(hi, c);
    }
    let clamped = clamp(history, lo, hi);
    // When the clamp had to pull history a long way, this pixel was likely
    // disoccluded/changed; lean on the current sample to avoid a trail.
    let ghost = clamp(length(clamped - history) * 6.0, 0.0, 1.0);
    let blend = mix(u.params.z, 1.0, ghost);
    return vec4(mix(clamped, current, blend), 1.0);
}
