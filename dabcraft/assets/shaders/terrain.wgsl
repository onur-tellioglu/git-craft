struct FrameUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,    // xyz camera world pos
    sky: vec4<f32>,       // rgb ambient sky color (linear), w day factor
    sun: vec4<f32>,       // xyz light dir (toward sun/moon), w 1=sun 0=moon
    sun_color: vec4<f32>, // rgb light radiance
    params: vec4<f32>,    // xy viewport px, z aerial km-per-meter
};

struct SectionInfo {
    origin: vec4<i32>,
};

struct ShadowUniform {
    mats: array<mat4x4<f32>, 3>,
    splits: vec4<f32>,  // cascade far view-distances
    texels: vec4<f32>,  // world texel size per cascade
};

@group(0) @binding(0) var<uniform> frame: FrameUniform;
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;
@group(1) @binding(1) var<storage, read> sections: array<SectionInfo>;
@group(2) @binding(0) var<uniform> shadow: ShadowUniform;
@group(2) @binding(1) var shadow_map: texture_depth_2d_array;
@group(2) @binding(2) var shadow_samp: sampler_comparison;
@group(3) @binding(0) var aerial_lut: texture_3d<f32>;
@group(3) @binding(1) var aerial_samp: sampler;

// Per-face: origin offset (added to voxel pos), U axis, V axis.
// Face order matches Rust: 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z.
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
// Ambient directional shade (the direct term has real NdotL now).
const FACE_SHADE = array<f32, 6>(0.8, 0.8, 1.0, 0.5, 0.6, 0.6);
const TORCH_COLOR = vec3(1.0, 0.62, 0.33);

// M2 palette indexed by the quad's texture field = block id;
// procedural textures replace this in M6.
const PALETTE = array<vec3<f32>, 13>(
    vec3(1.0, 0.0, 1.0),      //  0 air (never rendered; magenta = bug)
    vec3(0.35, 0.62, 0.22),   //  1 grass
    vec3(0.45, 0.32, 0.2),    //  2 dirt
    vec3(0.52, 0.52, 0.54),   //  3 stone
    vec3(0.86, 0.81, 0.58),   //  4 sand
    vec3(0.91, 0.93, 0.95),   //  5 snow grass
    vec3(0.19, 0.36, 0.68),   //  6 water (opaque until M5)
    vec3(0.42, 0.31, 0.19),   //  7 oak log
    vec3(0.23, 0.43, 0.14),   //  8 oak leaves
    vec3(0.32, 0.23, 0.14),   //  9 spruce log
    vec3(0.16, 0.3, 0.19),    // 10 spruce leaves
    vec3(0.27, 0.5, 0.21),    // 11 cactus
    vec3(0.95, 0.71, 0.3),    // 12 torch
);

const CORNER_UV = array<vec2<f32>, 4>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

const SHADOW_TEXEL: f32 = 1.0 / 2048.0;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) @interpolate(flat) face: u32,
    @location(2) ao: f32,
    // x = skylight, y = blocklight (constant across a greedy quad).
    @location(3) @interpolate(flat) light: vec2<f32>,
    @location(4) @interpolate(flat) albedo: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> VsOut {
    let quad = quads[vi / 4u];
    let flip = extractBits(quad.y, 31u, 1u);
    // AO diagonal flip: rotating the corner mapping by one turns the fixed
    // index pattern (0,1,2)(0,2,3) into triangles (1,2,3)(1,3,0) — the same
    // rectangle cut along the other diagonal.
    let corner = (vi + flip) % 4u;

    let x = f32(extractBits(quad.x, 0u, 6u));
    let y = f32(extractBits(quad.x, 6u, 6u));
    let z = f32(extractBits(quad.x, 12u, 6u));
    let face = extractBits(quad.x, 18u, 3u);
    let w = f32(extractBits(quad.x, 21u, 5u) + 1u);
    let h = f32(extractBits(quad.y, 0u, 5u) + 1u);
    let ao = f32(extractBits(quad.y, 5u + corner * 2u, 2u));
    let skylight = f32(extractBits(quad.y, 13u, 4u)) / 15.0;
    let blocklight = f32(extractBits(quad.y, 17u, 4u)) / 15.0;
    let tex = extractBits(quad.y, 21u, 10u);

    let uv = CORNER_UV[corner];
    let local = vec3(x, y, z) + FACE_ORIGIN[face] + FACE_U[face] * uv.x * w + FACE_V[face] * uv.y * h;
    let world = vec3<f32>(sections[slot].origin.xyz) + local;

    var out: VsOut;
    out.clip = frame.view_proj * vec4(world, 1.0);
    out.world_pos = world;
    out.face = face;
    out.ao = ao / 3.0;
    out.light = vec2(skylight, blocklight);
    out.albedo = PALETTE[min(tex, 12u)];
    return out;
}

// 5×5 PCF over the selected cascade; each tap is hardware 2×2 PCF, so the
// effective penumbra is ~6 texels. The wider kernel turns hard one-texel edges
// into a gradient, which stops the edge from flickering on/off under motion.
fn shadow_factor(world_pos: vec3<f32>, normal: vec3<f32>, view_dist: f32) -> f32 {
    var c: u32 = 3u;
    if view_dist < shadow.splits.x { c = 0u; }
    else if view_dist < shadow.splits.y { c = 1u; }
    else if view_dist < shadow.splits.z { c = 2u; }
    if c == 3u {
        return 1.0; // beyond the cascades: the skylight guard rules alone
    }
    // Normal-offset bias scaled by this cascade's texel footprint. Pushed out
    // far enough that flat lit faces never self-shadow (acne is the main
    // flicker source on a moving camera).
    let pos = world_pos + normal * shadow.texels[c] * 3.0;
    let p = shadow.mats[c] * vec4(pos, 1.0);
    let uv = vec2(p.x, -p.y) * 0.5 + 0.5;
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 1.0;
    }
    var sum = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let o = vec2(f32(dx), f32(dy)) * SHADOW_TEXEL;
            sum += textureSampleCompareLevel(shadow_map, shadow_samp, uv + o, c, p.z);
        }
    }
    return sum / 25.0;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let normal = FACE_NORMAL[in.face];
    let view_dist = length(in.world_pos - frame.camera.xyz);
    let ndotl = max(dot(normal, frame.sun.xyz), 0.0);

    // Flood-fill skylight gates the direct term beyond shadow range and
    // underground (spec §6): caves stay dark at noon, shafts of light need
    // actual sky exposure.
    let guard = smoothstep(0.0, 0.5, in.light.x);
    var shadow_f = 0.0;
    if ndotl > 0.0 && guard > 0.0 {
        shadow_f = shadow_factor(in.world_pos, normal, view_dist);
    }

    let ao = mix(0.35, 1.0, in.ao);
    let direct = frame.sun_color.rgb * ndotl * min(shadow_f, guard);
    let ambient = frame.sky.rgb * pow(in.light.x, 1.8) * FACE_SHADE[in.face] * ao;
    let torch = TORCH_COLOR * 1.4 * pow(in.light.y, 1.6) * FACE_SHADE[in.face] * ao;
    let lit = in.albedo * (direct + ambient + torch);
    // Aerial perspective: froxel slice indexed by exaggerated view distance.
    // 10.0 = AP_MAX_KM in sky_luts.wgsl.
    let screen_uv = in.clip.xy / frame.params.xy;
    let slice = clamp(view_dist * frame.params.z / 10.0, 0.0, 1.0);
    let ap = textureSampleLevel(aerial_lut, aerial_samp, vec3(screen_uv, slice), 0.0);
    return vec4(lit * ap.a + ap.rgb, 1.0);
}
