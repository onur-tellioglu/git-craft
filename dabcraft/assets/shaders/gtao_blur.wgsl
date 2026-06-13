// Depth-aware bilateral blur of the raw half-res AO. Weights neighbors by
// depth similarity so occlusion does not bleed across edges.

struct BlurUniform {
    params: vec4<f32>, // xy half-res px, z depth sigma, w unused
};

@group(0) @binding(0) var ao_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_2d<f32>;
@group(0) @binding(2) var<uniform> u: BlurUniform;

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

fn center_depth(px: vec2<i32>) -> f32 {
    return textureLoad(depth_tex, px * 2, 0).r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) f32 {
    let px = vec2<i32>(in.pos.xy);
    let dc = center_depth(px);
    var sum = 0.0;
    var wsum = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let p = px + vec2(dx, dy);
            let d = center_depth(p);
            let w = exp(-abs(d - dc) / u.params.z);
            sum += textureLoad(ao_tex, p, 0).r * w;
            wsum += w;
        }
    }
    return sum / max(wsum, 1e-4);
}
