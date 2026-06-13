// Final post pass: HDR offscreen target -> swapchain.
// Rung 0: plain blit. Rung 3 adds bloom mix, auto-exposure, and ACES here.
// The swapchain view is sRGB; this shader outputs LINEAR color and the
// hardware encodes on write.

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var hdr_samp: sampler;
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let hdr = textureSampleLevel(hdr_tex, hdr_samp, in.uv, 0.0).rgb;
    let bloom = textureSampleLevel(bloom_tex, hdr_samp, in.uv, 0.0).rgb;
    let color = mix(hdr, bloom, BLOOM_STRENGTH);
    return vec4(color, 1.0);
}
