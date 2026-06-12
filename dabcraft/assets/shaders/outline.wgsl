struct OutlineUniform {
    view_proj: mat4x4<f32>,
    // xyz = min corner of the targeted block (world space); w unused.
    block: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: OutlineUniform;

// 12 cube edges as 24 corner indices. Corner bit decode: x = bit 0,
// y = bit 1, z = bit 2.
const EDGES = array<u32, 24>(
    0u, 1u, 1u, 5u, 5u, 4u, 4u, 0u,  // bottom ring (y = 0)
    2u, 3u, 3u, 7u, 7u, 6u, 6u, 2u,  // top ring (y = 1)
    0u, 2u, 1u, 3u, 5u, 7u, 4u, 6u,  // verticals
);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let c = EDGES[vi];
    let corner = vec3<f32>(f32(c & 1u), f32((c >> 1u) & 1u), f32((c >> 2u) & 1u));
    // Inflate slightly around the cube center so the lines sit just off the
    // block faces instead of z-fighting them.
    let pos = u.block.xyz + vec3(0.5) + (corner - vec3(0.5)) * 1.004;
    return u.view_proj * vec4(pos, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4(0.05, 0.05, 0.05, 1.0);
}
