struct CameraUniform {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;

// Per-face: origin offset (added to voxel pos), U axis, V axis.
// Face order matches Rust: 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z.
// Invariant: cross(U, V) == outward face normal, so quads wind CCW seen from
// outside and survive backface culling. Quad w spans U, h spans V; AO corner
// order (0,0) (w,0) (w,h) (0,h) is defined in these same U/V axes.
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
// Minecraft-style face shading: +X, -X, +Y(top), -Y(bottom), +Z, -Z.
const FACE_SHADE = array<f32, 6>(0.8, 0.8, 1.0, 0.5, 0.6, 0.6);

// M1 block palette, indexed by the quad's texture field.
const PALETTE = array<vec3<f32>, 4>(
    vec3(1.0, 0.0, 1.0),      // 0 = air (never rendered; magenta = bug)
    vec3(0.35, 0.62, 0.22),   // 1 = grass
    vec3(0.45, 0.32, 0.2),    // 2 = dirt
    vec3(0.52, 0.52, 0.54),   // 3 = stone
);

// Corner order matches PackedQuad ao order: (0,0) (w,0) (w,h) (0,h).
const CORNER_UV = array<vec2<f32>, 4>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let quad = quads[vi / 4u];
    let flip = extractBits(quad.y, 31u, 1u);
    // AO diagonal flip: rotating the corner mapping by one turns the fixed
    // index pattern (0,1,2)(0,2,3) into triangles (1,2,3)(1,3,0) — the same
    // rectangle cut along the other diagonal. Positions and AO follow the
    // rotated corner, so geometry is identical and only the cut changes.
    let corner = (vi + flip) % 4u;

    let x = f32(extractBits(quad.x, 0u, 6u));
    let y = f32(extractBits(quad.x, 6u, 6u));
    let z = f32(extractBits(quad.x, 12u, 6u));
    let face = extractBits(quad.x, 18u, 3u);
    let w = f32(extractBits(quad.x, 21u, 5u) + 1u);
    let h = f32(extractBits(quad.y, 0u, 5u) + 1u);
    let ao = f32(extractBits(quad.y, 5u + corner * 2u, 2u));
    let skylight = f32(extractBits(quad.y, 13u, 4u));
    let tex = extractBits(quad.y, 21u, 10u);

    let uv = CORNER_UV[corner];
    let pos = vec3(x, y, z) + FACE_ORIGIN[face] + FACE_U[face] * uv.x * w + FACE_V[face] * uv.y * h;

    var out: VsOut;
    out.clip = camera.view_proj * vec4(pos, 1.0);
    let light = (skylight / 15.0) * FACE_SHADE[face] * mix(0.4, 1.0, ao / 3.0);
    out.color = PALETTE[min(tex, 3u)] * light;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
