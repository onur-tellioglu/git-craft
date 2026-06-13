// Jimenez 2014 (Advanced Warfare) bloom: 13-tap downsample with optional
// Karis average on the first level; 3x3 tent upsample drawn additively.

struct BloomUniform {
    texel: vec2<f32>,   // SOURCE texel size
    karis: f32,         // 1.0 only on the HDR -> mip0 downsample
    intensity: f32,     // tent upsample scale
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> u: BloomUniform;

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

fn s(uv: vec2<f32>, dx: f32, dy: f32) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv + vec2(dx, dy) * u.texel, 0.0).rgb;
}

fn karis_weight(c: vec3<f32>) -> f32 {
    return 1.0 / (1.0 + dot(c, vec3(0.2126, 0.7152, 0.0722)));
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let a = s(uv, -2.0, -2.0); let b = s(uv, 0.0, -2.0); let c = s(uv, 2.0, -2.0);
    let d = s(uv, -2.0, 0.0);  let e = s(uv, 0.0, 0.0);  let f = s(uv, 2.0, 0.0);
    let g = s(uv, -2.0, 2.0);  let h = s(uv, 0.0, 2.0);  let i = s(uv, 2.0, 2.0);
    let j = s(uv, -1.0, -1.0); let k = s(uv, 1.0, -1.0);
    let l = s(uv, -1.0, 1.0);  let m = s(uv, 1.0, 1.0);
    // Five overlapping 2x2 blocks: center half weight, corners eighth each.
    let b0 = (j + k + l + m) * 0.25;
    let b1 = (a + b + d + e) * 0.25;
    let b2 = (b + c + e + f) * 0.25;
    let b3 = (d + e + g + h) * 0.25;
    let b4 = (e + f + h + i) * 0.25;
    if u.karis > 0.5 {
        let w0 = karis_weight(b0); let w1 = karis_weight(b1); let w2 = karis_weight(b2);
        let w3 = karis_weight(b3); let w4 = karis_weight(b4);
        let col = b0 * w0 * 0.5 + (b1 * w1 + b2 * w2 + b3 * w3 + b4 * w4) * 0.125;
        let wsum = w0 * 0.5 + (w1 + w2 + w3 + w4) * 0.125;
        return vec4(col / max(wsum, 1e-4), 1.0);
    }
    return vec4(b0 * 0.5 + (b1 + b2 + b3 + b4) * 0.125, 1.0);
}

@fragment
fn fs_up(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    var col = s(uv, -1.0, -1.0) + s(uv, 1.0, -1.0) + s(uv, -1.0, 1.0) + s(uv, 1.0, 1.0);
    col += (s(uv, -1.0, 0.0) + s(uv, 1.0, 0.0) + s(uv, 0.0, -1.0) + s(uv, 0.0, 1.0)) * 2.0;
    col += s(uv, 0.0, 0.0) * 4.0;
    return vec4(col / 16.0 * u.intensity, 1.0);
}
