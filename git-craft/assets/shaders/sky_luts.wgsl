// Hillaire 2020 ("A Scalable and Production Ready Sky and Atmosphere
// Rendering Technique") LUT chain. Units: km; planet center at the origin;
// the camera sits on +Y at radius ground+altitude. Constants MUST mirror
// src/render/atmosphere.rs.

struct AtmUniform {
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,       // xyz world pos (meters), w altitude km
    sun: vec4<f32>,          // xyz toward the sun (world space)
    sun_radiance: vec4<f32>, // rgb top-of-atmosphere sun radiance
};

@group(0) @binding(0) var<uniform> atm: AtmUniform;
@group(0) @binding(1) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(2) var multiscatter_lut: texture_2d<f32>;
@group(0) @binding(3) var lut_samp: sampler;

@group(1) @binding(0) var out_transmittance: texture_storage_2d<rgba16float, write>;
@group(1) @binding(1) var out_multiscatter: texture_storage_2d<rgba16float, write>;
@group(1) @binding(2) var out_skyview: texture_storage_2d<rgba16float, write>;
@group(1) @binding(3) var out_aerial: texture_storage_3d<rgba16float, write>;

const GROUND_R: f32 = 6360.0;
const TOP_R: f32 = 6460.0;
const RAYLEIGH_SCATTER = vec3(5.802e-3, 13.558e-3, 33.1e-3);
const RAYLEIGH_H: f32 = 8.0;
const MIE_SCATTER: f32 = 3.996e-3;
const MIE_ABSORB: f32 = 4.4e-3;
const MIE_H: f32 = 1.2;
const OZONE_ABSORB = vec3(0.650e-3, 1.881e-3, 0.085e-3);
const OZONE_CENTER: f32 = 25.0;
const OZONE_HALF_WIDTH: f32 = 15.0;
const PI: f32 = 3.14159265;

const TRANSMITTANCE_SIZE = vec2(256.0, 64.0);
const SKYVIEW_SIZE = vec2(192.0, 108.0);
const AP_SIZE: f32 = 32.0;
// Far froxel slice distance in km. terrain.wgsl divides by the same value.
const AP_MAX_KM: f32 = 10.0;

struct Media {
    rayleigh: vec3<f32>,   // scattering
    mie: f32,              // scattering
    extinction: vec3<f32>,
};

fn media_at(h_in: f32) -> Media {
    let h = max(h_in, 0.0);
    let rayl = exp(-h / RAYLEIGH_H);
    let mie = exp(-h / MIE_H);
    let ozone = max(1.0 - abs(h - OZONE_CENTER) / OZONE_HALF_WIDTH, 0.0);
    var m: Media;
    m.rayleigh = RAYLEIGH_SCATTER * rayl;
    m.mie = MIE_SCATTER * mie;
    m.extinction = m.rayleigh + vec3((MIE_SCATTER + MIE_ABSORB) * mie) + OZONE_ABSORB * ozone;
    return m;
}

// Nearest positive hit with the origin-centered sphere; -1.0 when missed.
fn ray_sphere(origin: vec3<f32>, dir: vec3<f32>, radius: f32) -> f32 {
    let b = dot(origin, dir);
    let c = dot(origin, origin) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 { return -1.0; }
    let sq = sqrt(disc);
    if -b - sq > 0.0 { return -b - sq; }
    if -b + sq > 0.0 { return -b + sq; }
    return -1.0;
}

fn rayleigh_phase(c: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + c * c);
}

// Cornette-Shanks, g = 0.8.
fn mie_phase(c: f32) -> f32 {
    let g = 0.8;
    let g2 = g * g;
    return 3.0 * (1.0 - g2) * (1.0 + c * c)
        / (8.0 * PI * (2.0 + g2) * pow(1.0 + g2 - 2.0 * g * c, 1.5));
}

fn lut_uv(r: f32, mu: f32) -> vec2<f32> {
    return vec2(mu * 0.5 + 0.5, clamp((r - GROUND_R) / (TOP_R - GROUND_R), 0.0, 1.0));
}

fn sample_transmittance(r: f32, mu: f32) -> vec3<f32> {
    return textureSampleLevel(transmittance_lut, lut_samp, lut_uv(r, mu), 0.0).rgb;
}

fn sample_multiscatter(r: f32, mu_sun: f32) -> vec3<f32> {
    return textureSampleLevel(multiscatter_lut, lut_samp, lut_uv(r, mu_sun), 0.0).rgb;
}

fn march_transmittance(pos: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    if ray_sphere(pos, dir, GROUND_R) > 0.0 {
        return vec3(0.0);
    }
    let t_top = ray_sphere(pos, dir, TOP_R);
    if t_top <= 0.0 { return vec3(1.0); }
    let dt = t_top / 40.0;
    var depth = vec3(0.0);
    for (var i = 0u; i < 40u; i++) {
        let p = pos + dir * ((f32(i) + 0.5) * dt);
        depth += media_at(length(p) - GROUND_R).extinction * dt;
    }
    return exp(-depth);
}

@compute @workgroup_size(8, 8, 1)
fn cs_transmittance(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 256u || id.y >= 64u { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / TRANSMITTANCE_SIZE;
    let mu = uv.x * 2.0 - 1.0;
    let r = GROUND_R + uv.y * (TOP_R - GROUND_R);
    let pos = vec3(0.0, r, 0.0);
    let dir = vec3(sqrt(max(1.0 - mu * mu, 0.0)), mu, 0.0);
    textureStore(out_transmittance, vec2<i32>(id.xy), vec4(march_transmittance(pos, dir), 1.0));
}

// Hillaire's multi-scattering: per (sun zenith, altitude), integrate 2nd-order
// luminance L2 and transfer f over 64 sphere directions with an isotropic
// phase, then the geometric series Psi = L2 / (1 - f).
@compute @workgroup_size(8, 8, 1)
fn cs_multiscatter(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 32u || id.y >= 32u { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / 32.0;
    let mu_sun = uv.x * 2.0 - 1.0;
    let r = GROUND_R + uv.y * (TOP_R - GROUND_R);
    let pos = vec3(0.0, r, 0.0);
    let sun_dir = vec3(sqrt(max(1.0 - mu_sun * mu_sun, 0.0)), mu_sun, 0.0);

    var lum = vec3(0.0);
    var f_ms = vec3(0.0);
    let n = 8u;
    for (var a = 0u; a < n; a++) {
        for (var b = 0u; b < n; b++) {
            let theta = PI * (f32(a) + 0.5) / f32(n);
            let phi = 2.0 * PI * (f32(b) + 0.5) / f32(n);
            let dir = vec3(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi));
            let dw = sin(theta) * (PI / f32(n)) * (2.0 * PI / f32(n));

            var t_max = ray_sphere(pos, dir, TOP_R);
            let t_ground = ray_sphere(pos, dir, GROUND_R);
            if t_ground > 0.0 { t_max = t_ground; }
            if t_max <= 0.0 { continue; }
            let dt = t_max / 20.0;
            var throughput = vec3(1.0);
            for (var s = 0u; s < 20u; s++) {
                let p = pos + dir * ((f32(s) + 0.5) * dt);
                let pr = length(p);
                let m = media_at(pr - GROUND_R);
                let scatter = m.rayleigh + vec3(m.mie);
                let sun_t = sample_transmittance(pr, dot(p / pr, sun_dir));
                let step_t = exp(-m.extinction * dt);
                let inv_ext = 1.0 / max(m.extinction, vec3(1e-6));
                // Energy-conserving in-step integration (Hillaire eq. 6).
                lum += throughput * (scatter * sun_t - scatter * sun_t * step_t) * inv_ext
                    * (1.0 / (4.0 * PI)) * dw;
                f_ms += throughput * (scatter - scatter * step_t) * inv_ext
                    * (1.0 / (4.0 * PI)) * dw;
                throughput *= step_t;
            }
        }
    }
    let psi = lum / max(vec3(1.0) - f_ms, vec3(1e-4));
    textureStore(out_multiscatter, vec2<i32>(id.xy), vec4(psi, 1.0));
}

// Sky-view LUT addressing: u = world azimuth / 2pi; v = elevation with a
// square warp that concentrates texels at the horizon. sky.wgsl inverts this
// mapping — change BOTH or the sky tears.
fn skyview_elevation(v: f32) -> f32 {
    let c = v * 2.0 - 1.0;
    return sign(c) * c * c * 0.5 * PI;
}

fn march_scattering(pos: vec3<f32>, dir: vec3<f32>, sun_dir: vec3<f32>, steps: u32, t_cap: f32) -> vec4<f32> {
    var t_max = ray_sphere(pos, dir, TOP_R);
    let t_ground = ray_sphere(pos, dir, GROUND_R);
    if t_ground > 0.0 { t_max = t_ground; }
    if t_cap > 0.0 { t_max = min(t_max, t_cap); }
    if t_max <= 0.0 { return vec4(0.0, 0.0, 0.0, 1.0); }
    let cos_sun = dot(dir, sun_dir);
    let p_rayl = rayleigh_phase(cos_sun);
    let p_mie = mie_phase(cos_sun);
    let dt = t_max / f32(steps);
    var lum = vec3(0.0);
    var throughput = vec3(1.0);
    for (var i = 0u; i < steps; i++) {
        let p = pos + dir * ((f32(i) + 0.5) * dt);
        let pr = length(p);
        let m = media_at(pr - GROUND_R);
        let mu_sun = dot(p / pr, sun_dir);
        let s = (m.rayleigh * p_rayl + vec3(m.mie * p_mie)) * sample_transmittance(pr, mu_sun)
            + (m.rayleigh + vec3(m.mie)) * sample_multiscatter(pr, mu_sun);
        let step_t = exp(-m.extinction * dt);
        lum += throughput * (s - s * step_t) / max(m.extinction, vec3(1e-6));
        throughput *= step_t;
    }
    return vec4(lum, dot(throughput, vec3(1.0 / 3.0)));
}

@compute @workgroup_size(8, 8, 1)
fn cs_skyview(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 192u || id.y >= 108u { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / SKYVIEW_SIZE;
    let azimuth = (uv.x * 2.0 - 1.0) * PI;
    let elev = skyview_elevation(uv.y);
    let dir = vec3(cos(elev) * sin(azimuth), sin(elev), -cos(elev) * cos(azimuth));
    let pos = vec3(0.0, GROUND_R + max(atm.camera.w, 5e-4), 0.0);
    let result = march_scattering(pos, dir, atm.sun.xyz, 32u, -1.0);
    textureStore(out_skyview, vec2<i32>(id.xy), vec4(result.rgb * atm.sun_radiance.rgb, 1.0));
}

// Aerial-perspective froxels: in-scatter and transmittance from the camera
// to 32 exaggerated view distances per screen cell.
@compute @workgroup_size(4, 4, 4)
fn cs_aerial(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 32u || id.y >= 32u || id.z >= 32u { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / AP_SIZE;
    let ndc = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.5, 1.0);
    let world = atm.inv_view_proj * ndc;
    let dir = normalize(world.xyz / world.w - atm.camera.xyz);
    // Atmosphere is horizontally homogeneous: only altitude matters.
    let pos = vec3(0.0, GROUND_R + max(atm.camera.w, 5e-4), 0.0);
    let t_end = (f32(id.z) + 1.0) / AP_SIZE * AP_MAX_KM;
    let result = march_scattering(pos, dir, atm.sun.xyz, 16u, t_end);
    textureStore(out_aerial, vec3<i32>(id), vec4(result.rgb * atm.sun_radiance.rgb, result.a));
}
