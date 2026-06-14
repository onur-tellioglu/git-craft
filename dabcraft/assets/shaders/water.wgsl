// Transparent water (M5d): refraction (the seafloor seen through the surface),
// reflection (sky-view LUT now; screen-space in Stage B), fresnel blend, and a
// depth-based water tint. Drawn after the composited opaque scene into the same
// HDR target; no depth attachment — the opaque depth is sampled and the water
// fragment self-tests with discard, so a read-only depth alias isn't needed.
//
// Vertex stage mirrors terrain.wgsl's vertex pulling; FrameUniform must match
// terrain.wgsl / render/terrain.rs exactly.

struct FrameUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,    // xyz camera world pos
    sky: vec4<f32>,       // rgb ambient sky, w day factor
    sun: vec4<f32>,       // xyz light dir, w 1=sun 0=moon
    sun_color: vec4<f32>, // rgb light radiance
    params: vec4<f32>,    // xy viewport px, z aerial km/m
};

struct SectionInfo {
    origin: vec4<i32>,
};

struct WaterUniform {
    tint: vec4<f32>,   // rgb tint color, w fog density per meter of depth
    params: vec4<f32>, // x time, y fresnel F0, z reflection strength, w refraction offset
};

@group(0) @binding(0) var<uniform> frame: FrameUniform;
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;
@group(1) @binding(1) var<storage, read> sections: array<SectionInfo>;
@group(2) @binding(0) var scene_tex: texture_2d<f32>; // opaque scene (refraction/SSR source)
@group(2) @binding(1) var depth_tex: texture_2d<f32>; // opaque depth (manual test + fog)
@group(2) @binding(2) var skyview: texture_2d<f32>;   // reflection fallback
@group(2) @binding(3) var samp: sampler;
@group(2) @binding(4) var<uniform> water: WaterUniform;

const PI: f32 = 3.14159265;

const FACE_ORIGIN = array<vec3<f32>, 6>(
    vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 0.0),
);
const FACE_U = array<vec3<f32>, 6>(
    vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0),
    vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0),
    vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0),
);
const FACE_V = array<vec3<f32>, 6>(
    vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0),
    vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0),
    vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0),
);
const FACE_NORMAL = array<vec3<f32>, 6>(
    vec3(1.0, 0.0, 0.0), vec3(-1.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0), vec3(0.0, -1.0, 0.0),
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, -1.0),
);
const CORNER_UV = array<vec2<f32>, 4>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) @interpolate(flat) face: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> VsOut {
    let quad = quads[vi / 4u];
    let flip = extractBits(quad.y, 31u, 1u);
    let corner = (vi + flip) % 4u;

    let x = f32(extractBits(quad.x, 0u, 6u));
    let y = f32(extractBits(quad.x, 6u, 6u));
    let z = f32(extractBits(quad.x, 12u, 6u));
    let face = extractBits(quad.x, 18u, 3u);
    let w = f32(extractBits(quad.x, 21u, 5u) + 1u);
    let h = f32(extractBits(quad.y, 0u, 5u) + 1u);

    let uv = CORNER_UV[corner];
    let local = vec3(x, y, z) + FACE_ORIGIN[face] + FACE_U[face] * uv.x * w + FACE_V[face] * uv.y * h;
    let world = vec3<f32>(sections[slot].origin.xyz) + local;

    var out: VsOut;
    out.clip = frame.view_proj * vec4(world, 1.0);
    out.world_pos = world;
    out.face = face;
    return out;
}

// Inverse of sky_luts.wgsl's mapping (azimuth linear, elevation square warp).
fn skyview_uv(dir: vec3<f32>) -> vec2<f32> {
    let azimuth = atan2(dir.x, -dir.z);
    let elev = asin(clamp(dir.y, -1.0, 1.0));
    let c = sign(elev) * sqrt(abs(elev) / (0.5 * PI));
    return vec2(azimuth / (2.0 * PI) + 0.5, c * 0.5 + 0.5);
}

fn sky_reflection(dir: vec3<f32>) -> vec3<f32> {
    var d = dir;
    d.y = max(d.y, 0.02); // reflections never look below the horizon
    return textureSampleLevel(skyview, samp, skyview_uv(d), 0.0).rgb;
}

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

const SSR_STEPS: i32 = 48;
const SSR_MAX_DIST: f32 = 96.0;   // world meters the ray marches
const SSR_THICKNESS: f32 = 1.5;   // max depth gap counted as a hit (meters in NDC-ish)

// March the reflection ray in world space, projecting to screen each step and
// testing against the opaque depth. Returns rgb + a (a = hit confidence; 0 = miss).
fn ssr(origin: vec3<f32>, dir: vec3<f32>, jitter: f32) -> vec4<f32> {
    let step = SSR_MAX_DIST / f32(SSR_STEPS);
    var t = step * (0.5 + jitter);
    var prev_t = 0.0;
    for (var i = 0; i < SSR_STEPS; i++) {
        let p = origin + dir * t;
        let clip = frame.view_proj * vec4(p, 1.0);
        if clip.w <= 0.0 {
            break;
        }
        let ndc = clip.xyz / clip.w;
        let uv = vec2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
            break;
        }
        let scene_d = textureLoad(depth_tex, vec2<i32>(uv * frame.params.xy), 0).r;
        if scene_d < 1.0 && ndc.z > scene_d {
            // The ray is behind the depth surface. Refine between prev_t and t,
            // then accept if the crossing is a thin surface (not a far gap).
            var lo = prev_t;
            var hi = t;
            for (var k = 0; k < 6; k++) {
                let mid = (lo + hi) * 0.5;
                let mp = origin + dir * mid;
                let mc = frame.view_proj * vec4(mp, 1.0);
                let mndc = mc.xyz / mc.w;
                let muv = vec2(mndc.x * 0.5 + 0.5, 0.5 - mndc.y * 0.5);
                let md = textureLoad(depth_tex, vec2<i32>(muv * frame.params.xy), 0).r;
                if mndc.z > md { hi = mid; } else { lo = mid; }
            }
            let hp = origin + dir * hi;
            let hc = frame.view_proj * vec4(hp, 1.0);
            let hndc = hc.xyz / hc.w;
            let huv = vec2(hndc.x * 0.5 + 0.5, 0.5 - hndc.y * 0.5);
            let hd = textureLoad(depth_tex, vec2<i32>(huv * frame.params.xy), 0).r;
            // Reject far-gap crossings (ray passed through empty space behind a near edge).
            if hndc.z - hd > SSR_THICKNESS * 0.02 {
                return vec4(0.0);
            }
            // Fade near screen edges and as the ray gets long (both look wrong).
            let edge = min(min(huv.x, 1.0 - huv.x), min(huv.y, 1.0 - huv.y));
            let fade = smoothstep(0.0, 0.08, edge) * (1.0 - hi / SSR_MAX_DIST);
            return vec4(textureSampleLevel(scene_tex, samp, huv, 0.0).rgb, fade);
        }
        prev_t = t;
        t += step;
    }
    return vec4(0.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.clip.xy / frame.params.xy;
    let px = vec2<i32>(in.clip.xy);

    // Manual depth test: discard water behind the opaque scene (no depth attachment).
    let scene_depth = textureLoad(depth_tex, px, 0).r;
    if in.clip.z > scene_depth {
        discard;
    }

    let t = water.params.x;
    // Surface normal: flat for vertical faces, rippled for the top surface.
    var n = FACE_NORMAL[in.face];
    if in.face == 2u {
        let rx = sin(in.world_pos.x * 0.8 + t * 1.5) + sin(in.world_pos.z * 0.6 - t * 1.1);
        let rz = cos(in.world_pos.z * 0.7 + t * 1.3) + cos(in.world_pos.x * 0.5 - t * 0.9);
        n = normalize(vec3(rx * 0.06, 1.0, rz * 0.06));
    }

    let v = normalize(frame.camera.xyz - in.world_pos);
    let ndotv = max(dot(n, v), 1e-3);
    let f0 = water.params.y;
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - ndotv, 5.0);

    // Refraction: sample the opaque scene behind, nudged by the surface normal.
    let refr_uv = clamp(uv + n.xz * water.params.w, vec2(0.0), vec2(1.0));
    var refracted = textureSampleLevel(scene_tex, samp, refr_uv, 0.0).rgb;

    // Water tint deepens with the column depth behind the surface.
    var fog = 1.0;
    if scene_depth < 1.0 {
        let sndc = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_depth, 1.0);
        let sworld = frame.inv_view_proj * sndc;
        let scene_pos = sworld.xyz / sworld.w;
        let depth_m = distance(scene_pos, in.world_pos);
        fog = 1.0 - exp(-water.tint.w * depth_m);
    }
    refracted = mix(refracted, water.tint.rgb, fog);

    // Reflection: screen-space ray march, falling back to the sky-view LUT on a
    // miss / off-screen / edge fade.
    let r = reflect(-v, n);
    let hit = ssr(in.world_pos, r, hash12(in.clip.xy + t));
    var reflected = mix(sky_reflection(r), hit.rgb, hit.a);
    let glint = pow(max(dot(r, frame.sun.xyz), 0.0), 200.0) * frame.sun.w;
    reflected += frame.sun_color.rgb * glint * 3.0;

    let color = mix(refracted, reflected, fresnel * water.params.z);
    return vec4(color, 1.0);
}
