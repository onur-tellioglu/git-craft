// Auto-exposure: 256-bin log2-luminance histogram over the HDR target
// (stride 2), then a one-workgroup resolve that adapts a smoothed exposure.
// result[0] = exposure factor, result[1] = mean log2 luminance (debug).

struct ExposureUniform {
    dt: f32,
    min_log_lum: f32,   // -12.0
    inv_log_range: f32, // 1.0 / 18.0
    log_range: f32,     // 18.0
};

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> bins: array<atomic<u32>, 256>;
@group(0) @binding(2) var<storage, read_write> result: array<f32, 4>;
@group(0) @binding(3) var<uniform> u: ExposureUniform;

var<workgroup> local_bins: array<atomic<u32>, 256>;

fn bin_of(c: vec3<f32>) -> u32 {
    let lum = dot(c, vec3(0.2126, 0.7152, 0.0722));
    if lum < 1e-4 {
        return 0u; // black bin, excluded from the mean
    }
    let l = clamp((log2(lum) - u.min_log_lum) * u.inv_log_range, 0.0, 1.0);
    return u32(l * 254.0 + 1.0);
}

@compute @workgroup_size(16, 16, 1)
fn cs_histogram(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    atomicStore(&local_bins[li], 0u);
    workgroupBarrier();
    let dim = textureDimensions(hdr_tex);
    let px = id.xy * 2u;
    if px.x < dim.x && px.y < dim.y {
        let c = textureLoad(hdr_tex, vec2<i32>(px), 0).rgb;
        atomicAdd(&local_bins[bin_of(c)], 1u);
    }
    workgroupBarrier();
    atomicAdd(&bins[li], atomicLoad(&local_bins[li]));
}

var<workgroup> w_sum: array<f32, 256>;
var<workgroup> w_count: array<f32, 256>;

@compute @workgroup_size(256, 1, 1)
fn cs_resolve(@builtin(local_invocation_index) li: u32) {
    let count = f32(atomicLoad(&bins[li]));
    atomicStore(&bins[li], 0u); // zeroed for the next frame
    w_sum[li] = count * f32(li);
    w_count[li] = select(count, 0.0, li == 0u); // skip the black bin
    workgroupBarrier();
    for (var stride = 128u; stride > 0u; stride >>= 1u) {
        if li < stride {
            w_sum[li] += w_sum[li + stride];
            w_count[li] += w_count[li + stride];
        }
        workgroupBarrier();
    }
    if li == 0u {
        let total = max(w_count[0], 1.0);
        let mean_bin = w_sum[0] / total;
        let mean_log = (mean_bin - 1.0) / 254.0 * u.log_range + u.min_log_lum;
        let avg_lum = exp2(mean_log);
        let target_exp = clamp(0.115 / max(avg_lum, 1e-4), 0.03, 30.0);
        let prev = result[0];
        var exposure_val = target_exp;
        if prev > 0.0 {
            // Eye-style adaptation: darkening adapts faster than brightening.
            let rate = select(1.2, 2.5, target_exp < prev);
            exposure_val = prev + (target_exp - prev) * (1.0 - exp(-u.dt * rate));
        }
        result[0] = exposure_val;
        result[1] = mean_log;
    }
}
