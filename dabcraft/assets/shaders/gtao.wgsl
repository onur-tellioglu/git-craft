// Half-res horizon-based ambient occlusion. Reconstructs world position from
// the jittered depth buffer, reads the world normal from the g-buffer, and
// integrates the unoccluded horizon over NUM_DIRS screen-space directions with
// per-pixel + per-frame noise (TAA averages the noise out downstream).

struct GtaoUniform {
    inv_view_proj: mat4x4<f32>,
    params: vec4<f32>, // xy half-res px, z world radius, w frame index
    tune: vec4<f32>,   // x intensity, y depth bias, z max screen radius px, w power
};

@group(0) @binding(0) var depth_tex: texture_2d<f32>;
@group(0) @binding(1) var gbuf_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> u: GtaoUniform;

const NUM_DIRS: i32 = 4;
const NUM_STEPS: i32 = 6;
const PI: f32 = 3.14159265;

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

// Full-res depth sampled at a half-res UV.
fn load_depth(uv: vec2<f32>) -> f32 {
    let full = vec2<i32>(uv * u.params.xy * 2.0);
    return textureLoad(depth_tex, full, 0).r;
}

fn world_from(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec3(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let h = u.inv_view_proj * vec4(ndc, 1.0);
    return h.xyz / h.w;
}

// Interleaved gradient noise for the per-pixel rotation.
fn ign(px: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(px, vec2(0.06711056, 0.00583715))));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) f32 {
    // samp is declared for layout compatibility (future filtering use).
    let _samp_ref = samp;
    let px = vec2<i32>(in.pos.xy);
    let depth = textureLoad(depth_tex, vec2<i32>(in.uv * u.params.xy * 2.0), 0).r;
    if depth >= 1.0 {
        return 1.0; // sky: no occlusion
    }

    let center = world_from(in.uv, depth);
    let normal = normalize(textureLoad(gbuf_tex, vec2<i32>(in.uv * u.params.xy * 2.0), 0).rgb * 2.0 - 1.0);

    // Per-pixel + per-frame rotated direction set.
    let noise = ign(in.pos.xy + u.params.w * 5.588238);
    let radius_px = u.tune.z;
    let texel = 1.0 / u.params.xy;

    var ao = 0.0;
    for (var d = 0; d < NUM_DIRS; d++) {
        let angle = (f32(d) + noise) * (PI / f32(NUM_DIRS));
        let dir = vec2(cos(angle), sin(angle));
        var max_horizon = -1.0;
        for (var s = 1; s <= NUM_STEPS; s++) {
            let t = f32(s) / f32(NUM_STEPS);
            let suv = in.uv + dir * t * radius_px * texel;
            if any(suv < vec2(0.0)) || any(suv > vec2(1.0)) { continue; }
            let sd = load_depth(suv);
            if sd >= 1.0 { continue; }
            let sw = world_from(suv, sd);
            let v = sw - center;
            let dist = length(v);
            if dist < 1e-4 || dist > u.params.z { continue; }
            // Horizon = how far above the tangent plane this sample sits.
            let horizon = dot(normalize(v), normal);
            // Distance falloff so far samples occlude less.
            let falloff = clamp(1.0 - dist / u.params.z, 0.0, 1.0);
            max_horizon = max(max_horizon, horizon * falloff - u.tune.y);
        }
        ao += clamp(max_horizon, 0.0, 1.0);
    }
    ao = ao / f32(NUM_DIRS);
    let visibility = pow(clamp(1.0 - ao * u.tune.x, 0.0, 1.0), u.tune.w);
    return visibility;
}
