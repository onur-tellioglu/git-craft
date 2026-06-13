// Apply AO to the ambient term only: factor = 1 - ambientWeight*(1 - ao).
// ambientWeight (g-buffer alpha) is the fraction of pixel brightness from sky
// ambient, so direct sun and torch light are never darkened by occlusion.

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var gbuf_tex: texture_2d<f32>;
@group(0) @binding(2) var ao_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

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
    let hdr = textureLoad(hdr_tex, px, 0).rgb;
    let ambient_weight = textureLoad(gbuf_tex, px, 0).a;
    // Half-res AO, bilinearly upsampled.
    let ao = textureSampleLevel(ao_tex, samp, in.uv, 0.0).r;
    let factor = 1.0 - ambient_weight * (1.0 - ao);
    return vec4(hdr * factor, 1.0);
}
