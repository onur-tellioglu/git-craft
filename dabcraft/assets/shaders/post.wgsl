// Final post pass: HDR offscreen target -> swapchain.
// Rung 0: plain blit. Rung 3 adds bloom mix, auto-exposure, and ACES here.
// The swapchain view is sRGB; this shader outputs LINEAR color and the
// hardware encodes on write.

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var hdr_samp: sampler;
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
@group(0) @binding(3) var<storage, read> exposure: array<f32, 4>;

const BLOOM_STRENGTH: f32 = 0.06;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Fullscreen triangle: UVs (0,0) (2,0) (0,2) cover the screen once.
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

// Stephen Hill's ACES fit: sRGB -> ACEScg-ish input transform, RRT+ODT
// rational fit, output transform. Matrices are column-major (transposed
// from the HLSL original).
const ACES_IN = mat3x3<f32>(
    vec3(0.59719, 0.07600, 0.02840),
    vec3(0.35458, 0.90834, 0.13383),
    vec3(0.04823, 0.01566, 0.83777),
);
const ACES_OUT = mat3x3<f32>(
    vec3(1.60475, -0.10208, -0.00327),
    vec3(-0.53108, 1.10813, -0.07276),
    vec3(-0.07367, -0.00605, 1.07602),
);

fn rrt_odt_fit(v: vec3<f32>) -> vec3<f32> {
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return a / b;
}

fn aces(color: vec3<f32>) -> vec3<f32> {
    return clamp(ACES_OUT * rrt_odt_fit(ACES_IN * color), vec3(0.0), vec3(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let hdr = textureSampleLevel(hdr_tex, hdr_samp, in.uv, 0.0).rgb;
    let bloom = textureSampleLevel(bloom_tex, hdr_samp, in.uv, 0.0).rgb;
    let exposed = mix(hdr, bloom, BLOOM_STRENGTH) * max(exposure[0], 1e-3);
    return vec4(aces(exposed), 1.0);
}
