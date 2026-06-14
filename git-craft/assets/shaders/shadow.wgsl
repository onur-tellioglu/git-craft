// Depth-only cascade pass: the same vertex pulling as terrain.wgsl with a
// minimal vertex stage and no fragment stage (spec §6 pass 2).

struct CascadeUniform {
    view_proj: mat4x4<f32>,
};

struct SectionInfo {
    origin: vec4<i32>,
};

@group(0) @binding(0) var<uniform> cascade: CascadeUniform;
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;
@group(1) @binding(1) var<storage, read> sections: array<SectionInfo>;

// Identical tables to terrain.wgsl (no WGSL includes; keep in sync by hand).
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
const CORNER_UV = array<vec2<f32>, 4>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> @builtin(position) vec4<f32> {
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
    return cascade.view_proj * vec4(world, 1.0);
}
