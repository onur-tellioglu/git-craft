// Sky background: fullscreen triangle at depth 1.0 (LessEqual, no write),
// drawn after opaque terrain. Samples the sky-view LUT and adds a sun disc.
// The FrameUniform struct must match terrain.wgsl / render/terrain.rs.

struct FrameUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,
    sky: vec4<f32>,
    sun: vec4<f32>,       // xyz light dir, w 1=sun 0=moon
    sun_color: vec4<f32>, // rgb light radiance
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniform;
@group(1) @binding(0) var skyview: texture_2d<f32>;
@group(1) @binding(1) var sky_samp: sampler;

const PI: f32 = 3.14159265;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0); // z = far plane
    out.uv = uv;
    return out;
}

// Inverse of sky_luts.wgsl's mapping (azimuth linear, elevation square warp).
fn skyview_uv(dir: vec3<f32>) -> vec2<f32> {
    let azimuth = atan2(dir.x, -dir.z);
    let elev = asin(clamp(dir.y, -1.0, 1.0));
    let c = sign(elev) * sqrt(abs(elev) / (0.5 * PI));
    return vec2(azimuth / (2.0 * PI) + 0.5, c * 0.5 + 0.5);
}

struct FragOut {
    @location(0) color: vec4<f32>,
    @location(1) gbuf: vec4<f32>,
}

@fragment
fn fs_main(in: VsOut) -> FragOut {
    let ndc = vec4(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, 1.0, 1.0);
    let world = frame.inv_view_proj * ndc;
    let dir = normalize(world.xyz / world.w - frame.camera.xyz);
    var color = textureSampleLevel(skyview, sky_samp, skyview_uv(dir), 0.0).rgb;

    // Soften the sub-horizon band: the sky-view LUT goes dark just below the
    // horizon (ground-hitting rays carry no ground albedo). Fade those
    // directions toward the bright horizon color so distant terrain melts into
    // haze instead of a hard dark line. The blend saturates ~9 deg down, where
    // terrain covers the sky anyway.
    if dir.y < 0.0 {
        let horizon = textureSampleLevel(skyview, sky_samp, skyview_uv(vec3(dir.x, 0.0, dir.z)), 0.0).rgb;
        color = mix(color, horizon, clamp(-dir.y / 0.16, 0.0, 1.0));
    }

    // Sun disc (real sun only — frame.sun is the moon at night), angular
    // radius ~0.27 deg with a soft limb; sun_color already carries the
    // atmospheric transmittance, so the disc reddens at sunset for free.
    if frame.sun.w > 0.5 && dir.y > -0.1 {
        let d = dot(dir, frame.sun.xyz);
        let disc = smoothstep(cos(0.0055), cos(0.0035), d);
        color += frame.sun_color.rgb * 40.0 * disc;
    }
    var out: FragOut;
    out.color = vec4(color, 1.0);
    out.gbuf = vec4<f32>(0.0);
    return out;
}
