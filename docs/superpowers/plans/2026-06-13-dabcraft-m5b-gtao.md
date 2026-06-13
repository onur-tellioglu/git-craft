---
title: dabcraft M5b — GTAO (ground-truth ambient occlusion)
date: 2026-06-13
domain: world-layer
type: enhancement
priority: high
breaking: false
db-migration: false
rls-affecting: false
slice: null
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files: [dabcraft/src/render/targets.rs, dabcraft/src/render/gtao.rs, dabcraft/src/render/terrain.rs, dabcraft/src/render/atmosphere.rs, dabcraft/src/render/outline.rs, dabcraft/src/app.rs, dabcraft/assets/shaders/terrain.wgsl, dabcraft/assets/shaders/sky.wgsl, dabcraft/assets/shaders/outline.wgsl, dabcraft/assets/shaders/gtao.wgsl, dabcraft/assets/shaders/gtao_blur.wgsl, dabcraft/assets/shaders/composite.wgsl]
trigger-tasks-touched: []
shared-modules-touched: [dabcraft/src/render/targets.rs, dabcraft/src/app.rs]
---

# dabcraft M5b — GTAO Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add ground-truth ambient occlusion (GTAO) to the dabcraft forward renderer — half-res horizon-based AO that darkens only the *ambient* term (not direct sun), temporally stabilized by the existing TAA pass.

**Architecture:** The main forward pass gains a second color attachment — a packed G-buffer (`Rgba8Unorm`: `rgb` = world normal, `a` = ambient-brightness fraction). A half-res GTAO render pass reconstructs world position from the (jittered) depth buffer and the G-buffer normal, computes horizon-based occlusion with per-pixel + per-frame noise, and writes an `R8Unorm` AO texture. A depth-aware bilateral blur denoises it. A full-res composite pass darkens the HDR scene by `factor = 1 - ambientWeight*(1 - ao)` — so AO only attenuates ambient light, never the direct sun term — and writes a `composited` HDR target that TAA then consumes (TAA stabilizes the AO noise for free). Bloom/exposure/post are unchanged because they already read the TAA-resolved target downstream.

**Tech Stack:** Rust 2024, wgpu 29, WGSL. Fullscreen render passes (no compute / no storage textures) to stay TBDR-friendly and dodge storage-format caveats.

**Frame graph after this rung:** `luts → shadow0..2 → main(hdr + gbuf, jittered) → gtao(half) → gtao_blur(half) → composite(full) → taa → bloom → exposure → post → egui`.

**Spec deviations (documented, shipped code is authoritative):**
- Spec §6 calls GTAO a *compute* pass writing `RGB10A2` normals+ambient-fraction. We use a `Rgba8Unorm` G-buffer and *render* passes instead: `R8Unorm`/`R8` are render-attachment-capable everywhere, whereas `r8unorm` storage-texture writes need an optional WebGPU feature; fullscreen render passes also match the existing bloom/post pattern. The 2-bit alpha of `RGB10A2` is too coarse for a continuous ambient fraction, so we store it in the 8-bit alpha of `Rgba8Unorm`.
- AO is applied as a luminance-fraction attenuation (`1 - ambientWeight*(1-ao)`) rather than reconstructing the exact ambient color, so the composite needs only a scalar weight (no albedo / sky-color / aerial re-derivation).

**Validation:** Rendering correctness is validated via the F3 HUD per-pass GPU timers and visual inspection (project convention: rendering is never validated "by feel" but via the HUD/`--bench`, not unit tests). Pure helpers (half-res dims, uniform layout sizes, noise determinism) are unit-tested. Quality gates per task: `cargo test` + `cargo clippy --all-targets -- -D warnings`. `cargo fmt` is NOT a gate.

**Environment:** Rust via rustup — every shell needs `export PATH="$HOME/.cargo/bin:$PATH"` before `cargo`. Run cargo with `--manifest-path dabcraft/Cargo.toml` from the repo root (`/Users/onurtellioglu/Github/Minecraft`). No git remote — skip push/PR. Branch is already `feat/m5-shaders` (M5 merges as one milestone; do NOT merge to main in this rung).

---

## File Structure

- `dabcraft/src/render/targets.rs` — **modify**: add `GBUF_FORMAT`, `AO_FORMAT`; add `gbuf_view`, `ao_raw_view`, `ao_blur_view`, `composited_view` (+ `composited_texture` if needed), `half_size()` helper.
- `dabcraft/src/render/gtao.rs` — **create**: `GtaoPass` (AO + blur fullscreen passes), `CompositePass`, `GtaoUniform`, pure helpers `half_res(w,h)` and `ign_known` test hook.
- `dabcraft/assets/shaders/gtao.wgsl` — **create**: half-res horizon-AO fragment shader.
- `dabcraft/assets/shaders/gtao_blur.wgsl` — **create**: depth-aware bilateral blur.
- `dabcraft/assets/shaders/composite.wgsl` — **create**: AO→ambient composite.
- `dabcraft/assets/shaders/terrain.wgsl` — **modify**: fragment emits 2 targets (color + G-buffer).
- `dabcraft/assets/shaders/sky.wgsl`, `dabcraft/assets/shaders/outline.wgsl` — **modify**: emit a zeroed 2nd target so the MRT pipeline is valid.
- `dabcraft/src/render/terrain.rs`, `atmosphere.rs` (SkyPass), `outline.rs` — **modify**: pipelines gain the 2nd `ColorTargetState`.
- `dabcraft/src/app.rs` — **modify**: main pass 2nd attachment; insert GTAO/blur/composite encode; renumber `PASS_*`; rewire TAA input to `composited_view`; resize rebuilds.

---

### Task 1: G-buffer attachment (world normal + ambient weight) on the main pass

**Files:**
- Modify: `dabcraft/src/render/targets.rs`
- Modify: `dabcraft/assets/shaders/terrain.wgsl:154-180` (fragment), `dabcraft/assets/shaders/sky.wgsl` (fragment), `dabcraft/assets/shaders/outline.wgsl` (fragment)
- Modify: `dabcraft/src/render/terrain.rs:336` (targets array), `dabcraft/src/render/atmosphere.rs:495` (SkyPass targets), `dabcraft/src/render/outline.rs:91` (targets array)
- Modify: `dabcraft/src/app.rs:655-689` (main pass color attachments), `:865-873` + `:1004-1017` (resize), `:957` (target storage)

The main pass currently writes one color target (`hdr_view`). After this task it writes two: `@location(0)` HDR color (unchanged) and `@location(1)` the G-buffer. Every pipeline that draws in the main pass (terrain, sky, outline) must declare both targets or the render pass is invalid.

**G-buffer packing (`Rgba8Unorm`):** `rgb = normal*0.5 + 0.5` (world-space; axis-aligned voxel normals round-trip exactly), `a = ambientWeight` = the fraction of this pixel's brightness that came from the *sky ambient* term (so the composite can attenuate only that fraction). Sky/outline write `vec4(0.0)` — those pixels have `depth >= 1.0` (sky) or are overlays, and GTAO/composite skip them.

- [ ] **Step 1: Add G-buffer format + target to `RenderTargets`**

In `dabcraft/src/render/targets.rs`, add the format constant near `HDR_FORMAT` (line 3):

```rust
/// G-buffer: rgb = world normal (*0.5+0.5), a = ambient brightness fraction.
/// Rgba8Unorm (not the spec's RGB10A2): 8-bit alpha holds a continuous ambient
/// fraction that 2-bit RGB10A2 alpha cannot; render-attachment-capable.
pub const GBUF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
```

Add a `gbuf_view` field to `RenderTargets` (after `hdr_view`, line 17):

```rust
    /// G-buffer (normal + ambient weight), written by the main pass alongside HDR.
    pub gbuf_view: wgpu::TextureView,
```

In `RenderTargets::new`, create the texture right after the `hdr` texture (after line 47) and build its view, then add `gbuf_view` to the returned struct:

```rust
        let gbuf = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gbuffer normal+ambient"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GBUF_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
```

```rust
            gbuf_view: gbuf.create_view(&wgpu::TextureViewDescriptor::default()),
```

- [ ] **Step 2: Build to confirm the struct compiles**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo build --manifest-path dabcraft/Cargo.toml`
Expected: compiles (a warning about `app.rs` not yet using `gbuf_view` is fine until Step 5). Existing `cargo test` still passes.

- [ ] **Step 3: Emit the G-buffer from `terrain.wgsl`**

In `dabcraft/assets/shaders/terrain.wgsl`, replace the fragment signature + return (lines 154-180). Change `fn fs_main(in: VsOut) -> @location(0) vec4<f32>` to return a struct with both targets, and compute `ambientWeight` from the sky-ambient luminance vs total luminance:

```wgsl
struct FragOut {
    @location(0) color: vec4<f32>,
    @location(1) gbuf: vec4<f32>, // rgb = normal*0.5+0.5, a = ambient weight
}

@fragment
fn fs_main(in: VsOut) -> FragOut {
    let normal = FACE_NORMAL[in.face];
    let view_dist = length(in.world_pos - frame.camera.xyz);
    let ndotl = max(dot(normal, frame.sun.xyz), 0.0);

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
    let screen_uv = in.clip.xy / frame.params.xy;
    let slice = clamp(view_dist * frame.params.z / 10.0, 0.0, 1.0);
    let ap = textureSampleLevel(aerial_lut, aerial_samp, vec3(screen_uv, slice), 0.0);
    let color = lit * ap.a + ap.rgb;

    // Ambient weight = how much of the on-screen brightness is the sky-ambient
    // term (the only term GTAO attenuates). Direct sun + torch are excluded so
    // AO never smudges lit faces or torch-lit caves.
    let LUMA = vec3(0.2126, 0.7152, 0.0722);
    let amb_lum = dot(in.albedo * ambient * ap.a, LUMA);
    let tot_lum = dot(color, LUMA) + 1e-4;
    let ambient_weight = clamp(amb_lum / tot_lum, 0.0, 1.0);

    var out: FragOut;
    out.color = vec4(color, 1.0);
    out.gbuf = vec4(normal * 0.5 + 0.5, ambient_weight);
    return out;
}
```

(The `PALETTE` / `FACE_*` tables and the vertex stage are unchanged — leave them byte-identical; `block.rs` parses the PALETTE.)

- [ ] **Step 4: Emit a zeroed G-buffer from `sky.wgsl` and `outline.wgsl`**

In `dabcraft/assets/shaders/sky.wgsl`, change the fragment to output both targets. Find the `@fragment fn fs_main(...) -> @location(0) vec4<f32>` and wrap its returns:

```wgsl
struct FragOut {
    @location(0) color: vec4<f32>,
    @location(1) gbuf: vec4<f32>,
}
```
Change the signature to `-> FragOut` and replace each `return <expr>;` with:
```wgsl
    var out: FragOut;
    out.color = <expr>;
    out.gbuf = vec4<f32>(0.0);
    return out;
```
Do the identical transform in `dabcraft/assets/shaders/outline.wgsl`.

- [ ] **Step 5: Add the 2nd `ColorTargetState` to all three main-pass pipelines**

In `dabcraft/src/render/terrain.rs` (around line 336), the `targets:` array currently has one `Some(ColorTargetState { format: <HDR_FORMAT>, ... })`. Add a second entry:

```rust
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: crate::render::targets::HDR_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: crate::render::targets::GBUF_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
```

(Match the existing struct fields exactly — copy the first entry's `blend`/`write_mask`; the snippet above is the canonical shape.) Apply the same two-entry `targets` array in `dabcraft/src/render/atmosphere.rs:495` (SkyPass `build_pipeline`) and `dabcraft/src/render/outline.rs:91`.

- [ ] **Step 6: Add the 2nd color attachment to the main render pass**

In `dabcraft/src/app.rs`, the main pass (lines 655-689) has a single-element `color_attachments`. Bind `gbuf_view` as the second attachment. First capture it next to `hdr_view` (line 637 area):

```rust
        let hdr_view = &targets.hdr_view;
        let gbuf_view = &targets.gbuf_view;
```

Then make `color_attachments` two elements:

```rust
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: hdr_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: gbuf_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
```

- [ ] **Step 7: Build + run existing tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml --all-targets -- -D warnings`
Expected: all existing tests pass; clippy clean. (No new behavior is observable yet — the G-buffer is written but unread. A `cargo run --release` should look identical to before, proving the MRT pipelines validate and draw.)

- [ ] **Step 8: Commit**

```bash
git add dabcraft/src/render/targets.rs dabcraft/src/render/terrain.rs dabcraft/src/render/atmosphere.rs dabcraft/src/render/outline.rs dabcraft/assets/shaders/terrain.wgsl dabcraft/assets/shaders/sky.wgsl dabcraft/assets/shaders/outline.wgsl dabcraft/src/app.rs
git commit -m "feat: write a normal + ambient-weight g-buffer from the main pass (m5)"
```

---

### Task 2: Half-res GTAO render pass

**Files:**
- Modify: `dabcraft/src/render/targets.rs` (add `AO_FORMAT`, `ao_raw_view`, `ao_blur_view`, `half_size()`)
- Create: `dabcraft/src/render/gtao.rs` (`GtaoPass`, `GtaoUniform`, `half_res()`, `ign()` helper + tests)
- Create: `dabcraft/assets/shaders/gtao.wgsl`
- Modify: `dabcraft/src/render/mod.rs` (register the `gtao` module)
- Modify: `dabcraft/src/app.rs` (construct `GtaoPass`, encode it, renumber `PASS_*`, resize, hot-reload)

This task produces the raw half-res AO texture and wires the pass into the frame graph. The blur + composite (which make the AO visible on screen) land in Task 3 — after this task the AO texture is written but not yet consumed, validated by the new "gtao" HUD timer line appearing with a non-zero time and existing tests passing.

**GTAO algorithm (horizon-based, world-space):** For each output texel, reconstruct the center world position from the (jittered) depth buffer via `inv_view_proj`, read the world normal from the G-buffer. March `NUM_DIRS` screen-space directions (rotated per-pixel by interleaved-gradient noise and per-frame by a golden-angle offset so TAA averages a different sample set each frame); along each direction take `NUM_STEPS` samples within a screen-space radius, reconstruct each sample's world position, and track the maximum horizon elevation relative to the normal. AO is the averaged unoccluded fraction, contrast-shaped by a power. Sky texels (`depth >= 1.0`) output `1.0` (no occlusion).

- [ ] **Step 1: Add AO targets + half-res helper to `targets.rs`**

Add the format constant near `GBUF_FORMAT`:

```rust
/// Single-channel AO at half resolution. R8Unorm is render-attachment-capable
/// (unlike r8unorm storage), so GTAO runs as a fullscreen render pass.
pub const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
```

Add a free function (above `pub struct RenderTargets`):

```rust
/// Half resolution used by GTAO (and later volumetrics), clamped to >= 1.
pub fn half_size(width: u32, height: u32) -> (u32, u32) {
    ((width / 2).max(1), (height / 2).max(1))
}
```

Add fields to `RenderTargets`:

```rust
    /// Raw half-res GTAO output (before the bilateral blur).
    pub ao_raw_view: wgpu::TextureView,
    /// Blurred half-res GTAO output (read by the composite pass).
    pub ao_blur_view: wgpu::TextureView,
```

In `RenderTargets::new`, after the bloom block, create both half-res AO textures (a small helper closure keeps it DRY):

```rust
        let (hw, hh) = half_size(width, height);
        let make_ao = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: hw, height: hh, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: AO_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let ao_raw_view = make_ao("gtao raw");
        let ao_blur_view = make_ao("gtao blur");
```

Add `ao_raw_view` and `ao_blur_view` to the returned struct. Add a unit test:

```rust
    #[test]
    fn half_size_clamps_to_one() {
        assert_eq!(half_size(1512, 982), (756, 491));
        assert_eq!(half_size(1, 1), (1, 1));
        assert_eq!(half_size(0, 0), (1, 1), "never zero");
    }
```

- [ ] **Step 2: Create `gtao.rs` with the uniform + pure helpers + failing tests**

Create `dabcraft/src/render/gtao.rs`. Define the uniform (std140-friendly: mat4 then two vec4) and the pure helpers, plus tests that pin the layout and the noise determinism:

```rust
/// Uniform for the GTAO pass. Mirrors `GtaoUniform` in gtao.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GtaoUniform {
    /// Jittered inverse view-proj — matches the jittered depth buffer.
    pub inv_view_proj: [[f32; 4]; 4],
    /// x,y = half-res px dims; z = world-space sample radius; w = frame index.
    pub params: [f32; 4],
    /// x = intensity, y = depth-reject bias, z = max screen radius (px), w = power.
    pub tune: [f32; 4],
}

/// Half resolution for the AO buffer (mirror of targets::half_size, kept here
/// so the pass owns its own dispatch math without a cross-module call).
pub fn half_res(width: u32, height: u32) -> (u32, u32) {
    ((width / 2).max(1), (height / 2).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_is_96_bytes() {
        assert_eq!(std::mem::size_of::<GtaoUniform>(), 96);
    }

    #[test]
    fn half_res_matches_targets() {
        assert_eq!(half_res(1512, 982), (756, 491));
        assert_eq!(half_res(3, 3), (1, 1));
    }
}
```

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path dabcraft/Cargo.toml gtao`
Expected: FAIL (module not yet registered) → after Step 3 it passes.

- [ ] **Step 3: Register the module + build the `GtaoPass` struct**

In `dabcraft/src/render/mod.rs`, add `pub mod gtao;` alongside the other render modules.

Append the `GtaoPass` to `gtao.rs`. It owns a uniform buffer, a bind group layout (depth tex, gbuf tex, non-filtering sampler, uniform), one bind group, and a pipeline. Follow the `PostPass` pattern (fullscreen triangle, `immediate_size: 0`):

```rust
pub struct GtaoPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

impl GtaoPass {
    pub fn new(
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        gbuf_view: &wgpu::TextureView,
        shader_source: &str,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gtao bgl"),
            entries: &[
                // 0: depth (non-filterable, textureLoad)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 1: gbuffer (non-filterable, textureLoad)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 2: non-filtering sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                // 3: uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao uniform"),
            size: std::mem::size_of::<GtaoUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gtao sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_group =
            Self::build_bind_group(device, &layout, depth_view, gbuf_view, &sampler, &uniform);
        let pipeline = Self::build_pipeline(device, &layout, shader_source);
        Self { pipeline, layout, bind_group, uniform, sampler }
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        depth_view: &wgpu::TextureView,
        gbuf_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(gbuf_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
            ],
        })
    }

    fn build_pipeline(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gtao shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gtao pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gtao pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::render::targets::AO_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn prepare(&self, queue: &wgpu::Queue, u: &GtaoUniform) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(u));
    }

    pub fn rebuild_bind_group(
        &mut self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        gbuf_view: &wgpu::TextureView,
    ) {
        self.bind_group = Self::build_bind_group(
            device, &self.layout, depth_view, gbuf_view, &self.sampler, &self.uniform,
        );
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.pipeline = Self::build_pipeline(device, &self.layout, shader_source);
    }

    /// Render the half-res AO into `ao_raw_view`.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        ao_raw_view: &wgpu::TextureView,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gtao"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ao_raw_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
}
```

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path dabcraft/Cargo.toml gtao`
Expected: PASS (3 gtao tests).

- [ ] **Step 4: Write `gtao.wgsl`**

Create `dabcraft/assets/shaders/gtao.wgsl`:

```wgsl
// Half-res horizon-based ambient occlusion. Reconstructs world position from
// the jittered depth buffer, reads the world normal from the g-buffer, and
// integrates the unoccluded horizon over NUM_DIRS screen-space directions with
// per-pixel + per-frame noise (TAA averages the noise out downstream).

struct GtaoUniform {
    inv_view_proj: mat4x4<f32>,
    params: vec4<f32>, // xy half-res px, z world radius, w frame index
    tune: vec4<f32>,   // x intensity, y depth bias, z max screen radius px, w power
};

@group(0) @binding(0) var depth_tex: texture_2d<f32>;
@group(0) @binding(1) var gbuf_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> u: GtaoUniform;

const NUM_DIRS: i32 = 4;
const NUM_STEPS: i32 = 6;
const PI: f32 = 3.14159265;

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

// Full-res depth sampled at a half-res UV.
fn load_depth(uv: vec2<f32>) -> f32 {
    let full = vec2<i32>(uv * u.params.xy * 2.0);
    return textureLoad(depth_tex, full, 0).r;
}

fn world_from(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec3(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let h = u.inv_view_proj * vec4(ndc, 1.0);
    return h.xyz / h.w;
}

// Interleaved gradient noise for the per-pixel rotation.
fn ign(px: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(px, vec2(0.06711056, 0.00583715))));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) f32 {
    let px = vec2<i32>(in.pos.xy);
    let depth = textureLoad(depth_tex, vec2<i32>(in.uv * u.params.xy * 2.0), 0).r;
    if depth >= 1.0 {
        return 1.0; // sky: no occlusion
    }

    let center = world_from(in.uv, depth);
    let normal = normalize(textureLoad(gbuf_tex, vec2<i32>(in.uv * u.params.xy * 2.0), 0).rgb * 2.0 - 1.0);

    // Per-pixel + per-frame rotated direction set.
    let noise = ign(in.pos.xy + u.params.w * 5.588238);
    let radius_px = u.tune.z;
    let texel = 1.0 / u.params.xy;

    var ao = 0.0;
    for (var d = 0; d < NUM_DIRS; d++) {
        let angle = (f32(d) + noise) * (PI / f32(NUM_DIRS));
        let dir = vec2(cos(angle), sin(angle));
        var max_horizon = -1.0;
        for (var s = 1; s <= NUM_STEPS; s++) {
            let t = f32(s) / f32(NUM_STEPS);
            let suv = in.uv + dir * t * radius_px * texel;
            if any(suv < vec2(0.0)) || any(suv > vec2(1.0)) { continue; }
            let sd = load_depth(suv);
            if sd >= 1.0 { continue; }
            let sw = world_from(suv, sd);
            let v = sw - center;
            let dist = length(v);
            if dist < 1e-4 || dist > u.params.z { continue; }
            // Horizon = how far above the tangent plane this sample sits.
            let horizon = dot(normalize(v), normal);
            // Distance falloff so far samples occlude less.
            let falloff = clamp(1.0 - dist / u.params.z, 0.0, 1.0);
            max_horizon = max(max_horizon, horizon * falloff - u.tune.y);
        }
        ao += clamp(max_horizon, 0.0, 1.0);
    }
    ao = ao / f32(NUM_DIRS);
    let visibility = pow(clamp(1.0 - ao * u.tune.x, 0.0, 1.0), u.tune.w);
    return visibility;
}
```

- [ ] **Step 5: Renumber the `PASS_*` constants in `app.rs`**

In `dabcraft/src/app.rs` (lines 18-28), expand `PASS_LABELS` to 11 entries and insert the two new consts, renumbering everything downstream of `PASS_MAIN`:

```rust
const PASS_LABELS: &[&str] = &[
    "luts", "shadow0", "shadow1", "shadow2", "main", "gtao", "composite", "taa", "bloom",
    "exposure", "post",
];
const PASS_LUTS: usize = 0;
const PASS_SHADOW0: usize = 1;
const PASS_MAIN: usize = 4;
const PASS_GTAO: usize = 5;
const PASS_COMPOSITE: usize = 6;
const PASS_TAA: usize = 7;
const PASS_BLOOM: usize = 8;
const PASS_EXPOSURE: usize = 9;
const PASS_POST: usize = 10;
```

(`PASS_COMPOSITE` is unused until Task 3 — add `#[allow(dead_code)]` above it if `-D warnings` complains, and remove the allow in Task 3.)

- [ ] **Step 6: Construct, prepare, and encode the GTAO pass in `app.rs`**

Add a `gtao` field to the app struct (near `taa: Option<...>`): `gtao: Option<crate::render::gtao::GtaoPass>,` and initialize it to `None` in the constructor.

In the setup block (near where TAA is built, lines 948-956), construct it:

```rust
        let gtao_src =
            std::fs::read_to_string(shader_path("gtao.wgsl")).expect("gtao.wgsl missing");
        self.gtao = Some(crate::render::gtao::GtaoPass::new(
            &gpu.device,
            depth_sample_view_ref,
            &targets.gbuf_view,
            &gtao_src,
        ));
```

Note: `targets` is moved into `self.targets` at line 957, so construct `gtao` *before* that move (it borrows `targets.gbuf_view`), same as the TAA construction.

In `render()`, prepare the uniform alongside the TAA uniform (after line 627):

```rust
        if let Some(gtao) = self.gtao.as_ref() {
            let (hw, hh) = crate::render::gtao::half_res(gpu.config.width, gpu.config.height);
            gtao.prepare(&gpu.queue, &crate::render::gtao::GtaoUniform {
                inv_view_proj: jittered_vp.inverse().to_cols_array_2d(),
                params: [hw as f32, hh as f32, 1.5, self.frame_index as f32],
                tune: [1.4, 0.02, 48.0, 1.5],
            });
        }
```

Encode it right after the main pass closes (after line 689, before the TAA encode):

```rust
        let gtao_writes = self.timer.as_ref().and_then(|t| t.render_writes(PASS_GTAO));
        if let (Some(gtao), Some(targets)) = (self.gtao.as_ref(), self.targets.as_ref()) {
            gtao.encode(&mut encoder, &targets.ao_raw_view, gtao_writes);
        }
```

- [ ] **Step 7: Resize + hot-reload wiring**

In the resize handler (after the TAA `rebuild_bind_groups` call, ~line 1025), rebuild the GTAO bind group:

```rust
                    if let (Some(gtao), Some(targets), Some(depth_sv)) =
                        (self.gtao.as_mut(), self.targets.as_ref(), self.depth_sample_view.as_ref())
                    {
                        gtao.rebuild_bind_group(&gpu.device, depth_sv, &targets.gbuf_view);
                    }
```

Register the hot-reload watch (near line 886): `shaders.watch("gtao", shader_path("gtao.wgsl"));` and add a swap arm in the hot-reload match block (near lines 392-433) mirroring the `taa` arm:

```rust
                        "gtao" => {
                            if let Some(g) = self.gtao.as_mut() {
                                g.swap_shader(&gpu.device, &source);
                            }
                        }
```

- [ ] **Step 8: Build, test, smoke-run**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml --all-targets -- -D warnings`
Expected: all pass, clippy clean. A `cargo build --release` succeeds. Visual is still unchanged (AO unread); the F3 HUD now lists a "gtao" timer with a small non-zero ms.

- [ ] **Step 9: Commit**

```bash
git add dabcraft/src/render/targets.rs dabcraft/src/render/gtao.rs dabcraft/src/render/mod.rs dabcraft/assets/shaders/gtao.wgsl dabcraft/src/app.rs
git commit -m "feat: render half-res horizon AO into a g-buffer-fed target (m5)"
```

---

### Task 3: Bilateral blur + composite (apply AO to ambient), rewire TAA input

**Files:**
- Modify: `dabcraft/src/render/targets.rs` (add `composited_view` + `composited_texture`)
- Modify: `dabcraft/src/render/gtao.rs` (add `BlurPass` + `CompositePass`)
- Create: `dabcraft/assets/shaders/gtao_blur.wgsl`, `dabcraft/assets/shaders/composite.wgsl`
- Modify: `dabcraft/src/render/taa.rs` (read `composited_view` instead of `hdr_view`)
- Modify: `dabcraft/src/app.rs` (construct/encode blur + composite, rewire TAA, resize, hot-reload)

This task makes the AO visible: the blur denoises the half-res AO, and the composite darkens the HDR scene's ambient term by it. The composite writes a new full-res `composited` HDR target, which becomes TAA's input — so TAA temporally stabilizes the AO. After this task, ambient occlusion is on screen: contact shadows in corners, under overhangs, between blocks.

- [ ] **Step 1: Add the `composited` target to `targets.rs`**

Add fields:

```rust
    /// AO-composited HDR (main color × ambient-occlusion factor). TAA reads this.
    pub composited_view: wgpu::TextureView,
```

In `RenderTargets::new`, create it at full res with `HDR_FORMAT` (RENDER_ATTACHMENT | TEXTURE_BINDING) and add to the struct:

```rust
        let composited = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ao composited hdr"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
```
```rust
            composited_view: composited.create_view(&wgpu::TextureViewDescriptor::default()),
```

- [ ] **Step 2: Write `gtao_blur.wgsl` (depth-aware bilateral blur)**

Create `dabcraft/assets/shaders/gtao_blur.wgsl`. A single 5×5-ish cross bilateral at half-res, weighting taps by depth similarity so AO does not bleed across silhouettes:

```wgsl
// Depth-aware bilateral blur of the raw half-res AO. Weights neighbors by
// depth similarity so occlusion does not bleed across edges.

struct BlurUniform {
    params: vec4<f32>, // xy half-res px, z depth sigma, w unused
};

@group(0) @binding(0) var ao_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> u: BlurUniform;

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
```

- [ ] **Step 3: Write `composite.wgsl` (apply AO to the ambient fraction)**

Create `dabcraft/assets/shaders/composite.wgsl`:

```wgsl
// Apply AO to the ambient term only: factor = 1 - ambientWeight*(1 - ao).
// ambientWeight (g-buffer alpha) is the fraction of pixel brightness from sky
// ambient, so direct sun and torch light are never darkened by occlusion.

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var gbuf_tex: texture_2d<f32>;
@group(0) @binding(2) var ao_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.pos.xy);
    let hdr = textureLoad(hdr_tex, px, 0).rgb;
    let ambient_weight = textureLoad(gbuf_tex, px, 0).a;
    // Half-res AO, bilinearly upsampled.
    let ao = textureSampleLevel(ao_tex, samp, in.uv, 0.0).r;
    let factor = 1.0 - ambient_weight * (1.0 - ao);
    return vec4(hdr * factor, 1.0);
}
```

- [ ] **Step 4: Add `BlurPass` and `CompositePass` to `gtao.rs`**

Append two structs to `gtao.rs`, both fullscreen render passes following the same `build_pipeline` shape as `GtaoPass`.

`BlurPass`: bind group = `ao_raw` tex (binding 0), depth tex (1), non-filtering sampler (2), uniform `BlurUniform { params: [f32;4] }` (3). Output format `AO_FORMAT`. Methods: `new(device, ao_raw_view, depth_view, shader_source)`, `prepare(queue, params: [f32;4])`, `rebuild_bind_group(device, ao_raw_view, depth_view)`, `swap_shader`, `encode(encoder, ao_blur_view, timestamp_writes)` (LoadOp::Clear WHITE, draws 0..3). Define:

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BlurUniform {
    pub params: [f32; 4], // xy half-res px, z depth sigma, w unused
}
```

`CompositePass`: bind group = `hdr` tex (0), `gbuf` tex (1), `ao_blur` tex (2), filtering sampler (3, so the half-res AO upsamples bilinearly). No uniform. Output format `HDR_FORMAT`. Methods: `new(device, hdr_view, gbuf_view, ao_blur_view, shader_source)`, `rebuild_bind_group(device, hdr_view, gbuf_view, ao_blur_view)`, `swap_shader`, `encode(encoder, composited_view, timestamp_writes)`.

Add layout-size tests:

```rust
    #[test]
    fn blur_uniform_is_16_bytes() {
        assert_eq!(std::mem::size_of::<BlurUniform>(), 16);
    }
```

Note: the composite's sampler must be `Filtering` and its `gbuf`/`hdr`/`ao` textures `filterable: true` only for the AO; `hdr` and `gbuf` are read via `textureLoad` (integer px) so they can stay non-filterable — but a bind group cannot mix a `Filtering` sampler with a non-filterable texture it samples. Since `hdr`/`gbuf` use `textureLoad` (not the sampler), declare them `filterable: false` and the AO texture `filterable: true`, and split into two samplers? Simpler: declare all three textures `filterable: true` (HDR_FORMAT and Rgba8Unorm are both filterable formats) and use one `Filtering` sampler — `textureLoad` ignores the sampler, so this is valid and avoids a second sampler. Use that.

- [ ] **Step 5: Rewire TAA to read `composited_view`**

In `dabcraft/src/render/taa.rs`, every place the current-frame texture is bound it currently uses `targets.hdr_view` (lines ~90, ~99, ~262, ~271). Change all four to `targets.composited_view`. The neighborhood clamp and reprojection are unaffected — TAA simply consumes the AO-composited HDR as its "current" frame. Update the module doc comment to say it reads the AO-composited HDR.

- [ ] **Step 6: Construct + encode blur and composite in `app.rs`**

Add fields: `blur: Option<crate::render::gtao::BlurPass>,` and `composite: Option<crate::render::gtao::CompositePass>,` (init `None`).

Construct both right after the GTAO pass construction (before `self.targets = Some(targets)`):

```rust
        let blur_src =
            std::fs::read_to_string(shader_path("gtao_blur.wgsl")).expect("gtao_blur.wgsl missing");
        self.blur = Some(crate::render::gtao::BlurPass::new(
            &gpu.device,
            &targets.ao_raw_view,
            depth_sample_view_ref,
            &blur_src,
        ));
        let composite_src =
            std::fs::read_to_string(shader_path("composite.wgsl")).expect("composite.wgsl missing");
        self.composite = Some(crate::render::gtao::CompositePass::new(
            &gpu.device,
            &targets.hdr_view,
            &targets.gbuf_view,
            &targets.ao_blur_view,
            &composite_src,
        ));
```

Prepare the blur uniform in `render()` (next to the GTAO prepare):

```rust
        if let Some(blur) = self.blur.as_ref() {
            let (hw, hh) = crate::render::gtao::half_res(gpu.config.width, gpu.config.height);
            blur.prepare(&gpu.queue, [hw as f32, hh as f32, 0.0015, 0.0]);
        }
```

Encode blur then composite, right after the GTAO encode and before the TAA encode. Remove the `#[allow(dead_code)]` on `PASS_COMPOSITE` from Task 2:

```rust
        if let (Some(blur), Some(targets)) = (self.blur.as_ref(), self.targets.as_ref()) {
            blur.encode(&mut encoder, &targets.ao_blur_view, None);
        }
        let comp_writes = self.timer.as_ref().and_then(|t| t.render_writes(PASS_COMPOSITE));
        if let (Some(composite), Some(targets)) = (self.composite.as_ref(), self.targets.as_ref()) {
            composite.encode(&mut encoder, &targets.composited_view, comp_writes);
        }
```

(The blur shares the GTAO timer budget; it is left untimed here to keep one HUD line per concept — note this in the commit. If a separate timing is wanted later, give it its own slot.)

- [ ] **Step 7: Resize wiring for blur + composite**

In the resize handler, after the GTAO `rebuild_bind_group`:

```rust
                    if let (Some(blur), Some(targets), Some(depth_sv)) =
                        (self.blur.as_mut(), self.targets.as_ref(), self.depth_sample_view.as_ref())
                    {
                        blur.rebuild_bind_group(&gpu.device, &targets.ao_raw_view, depth_sv);
                    }
                    if let (Some(composite), Some(targets)) =
                        (self.composite.as_mut(), self.targets.as_ref())
                    {
                        composite.rebuild_bind_group(
                            &gpu.device,
                            &targets.hdr_view,
                            &targets.gbuf_view,
                            &targets.ao_blur_view,
                        );
                    }
```

(TAA's `rebuild_bind_groups` already runs in the resize handler and now picks up `composited_view` automatically because taa.rs reads that field.)

- [ ] **Step 8: Hot-reload for blur + composite**

Register watches: `shaders.watch("gtao_blur", shader_path("gtao_blur.wgsl"));` and `shaders.watch("composite", shader_path("composite.wgsl"));`. Add swap arms mirroring the `gtao` arm for `"gtao_blur"` → `self.blur` and `"composite"` → `self.composite`.

- [ ] **Step 9: Build, test, smoke-run, eyeball**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml --all-targets -- -D warnings`
Expected: all pass, clippy clean. `cargo run --release` now shows ambient occlusion: soft darkening in block corners, under tree canopies, in crevices — and it should be stable under motion (TAA resolves the half-res noise). Direct sun-lit flat faces stay full-bright (AO only touches ambient). The F3 HUD lists "gtao" and "composite" timers.

- [ ] **Step 10: Commit**

```bash
git add dabcraft/src/render/targets.rs dabcraft/src/render/gtao.rs dabcraft/src/render/taa.rs dabcraft/assets/shaders/gtao_blur.wgsl dabcraft/assets/shaders/composite.wgsl dabcraft/src/app.rs
git commit -m "feat: blur and composite GTAO onto the ambient term, fed through TAA (m5)"
```

---

### Task 4: AO debug toggle + GPU-budget validation

**Files:**
- Modify: `dabcraft/assets/shaders/composite.wgsl` (debug branch)
- Modify: `dabcraft/src/render/gtao.rs` (`CompositePass` gains a small debug uniform)
- Modify: `dabcraft/src/app.rs` (a key toggles the debug mode; feed it to composite)

A debug view that outputs the raw AO factor as grayscale makes tuning the radius/intensity/power constants tractable (and lets the user point at exactly what looks wrong). It is a single uniform flag read by the composite shader. This task also validates the GPU budget via the F3 HUD.

- [ ] **Step 1: Add a debug uniform to the composite shader**

In `dabcraft/assets/shaders/composite.wgsl`, add a uniform and a debug branch. Add the binding (binding 4) and `struct CompUniform { flags: vec4<f32> }` (`flags.x`: 0 = normal, 1 = AO only):

```wgsl
@group(0) @binding(4) var<uniform> c: CompUniform;
```
```wgsl
struct CompUniform { flags: vec4<f32>; };
```
At the end of `fs_main`, before the normal return:

```wgsl
    if c.flags.x > 0.5 {
        return vec4(vec3(ao), 1.0); // AO debug view
    }
```

- [ ] **Step 2: Add the debug uniform to `CompositePass`**

In `gtao.rs`, give `CompositePass` a `uniform: wgpu::Buffer` (16 bytes), add binding 4 (uniform, FRAGMENT) to its layout and bind group, and a `set_debug(&self, queue, on: bool)` method:

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CompUniform {
    pub flags: [f32; 4],
}
```
```rust
    pub fn set_debug(&self, queue: &wgpu::Queue, on: bool) {
        let u = CompUniform { flags: [if on { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0] };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
    }
```

Create the buffer in `new` (init to zeros) and include it in `build_bind_group`. Add a test `comp_uniform_is_16_bytes`.

- [ ] **Step 3: Wire a toggle key in `app.rs`**

Add a `gtao_debug: bool` field (init `false`). In the keyboard handler (near the F3/H block, ~line 1064), add an arm — use `KeyCode::KeyG`:

```rust
                    PhysicalKey::Code(KeyCode::KeyG) => {
                        self.gtao_debug = !self.gtao_debug;
                    }
```

In `render()`, push the flag to the composite each frame (next to the other `prepare` calls):

```rust
        if let Some(composite) = self.composite.as_ref() {
            composite.set_debug(&gpu.queue, self.gtao_debug);
        }
```

- [ ] **Step 4: Build + test**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path dabcraft/Cargo.toml && cargo clippy --manifest-path dabcraft/Cargo.toml --all-targets -- -D warnings`
Expected: all pass, clippy clean.

- [ ] **Step 5: GPU-budget validation (HUD)**

Run: `cargo run --release --manifest-path dabcraft/Cargo.toml`. Press Fn+F3 (or H) for the HUD. Confirm:
- "gtao" + "composite" timers appear with sane values (gtao should be ~0.5–1.0 ms per the spec budget, composite a fraction of a ms).
- Total per-pass GPU time stays within the 8.3 ms / 120 fps budget (the shadow draws were already noted near budget; if total now exceeds it, record the numbers — a shadow-pass perf pass is a separate M5b item, not a blocker for GTAO correctness).
- Press G: the screen switches to the grayscale AO view (white = unoccluded, dark = occluded corners/crevices). Press G again to return.
- Day-cycle sweep: AO is stable under camera motion and time-of-day change; no crawling/boiling in the AO; lit faces are not darkened by AO.

Record the observed gtao/composite ms and total in the commit body.

- [ ] **Step 6: Commit**

```bash
git add dabcraft/assets/shaders/composite.wgsl dabcraft/src/render/gtao.rs dabcraft/src/app.rs
git commit -m "feat: add a G-key AO debug view and validate the GTAO GPU budget (m5)"
```

---

## Self-Review

**Spec coverage (§6 pass 3/4/6):**
- "normals+ambient-fraction (Store)" → Task 1 G-buffer (`Rgba8Unorm`, normal in rgb, ambient weight in a). ✓ (format deviation documented).
- "GTAO half-res + blur" → Task 2 (half-res horizon AO) + Task 3 (bilateral blur). ✓
- "apply GTAO to ambient fraction" → Task 3 composite (`1 - ambientWeight*(1-ao)`). ✓ AO never touches the direct sun term (excluded from `ambientWeight`). ✓
- "wgpu timestamp queries measure every pass" → Task 2/3 add `PASS_GTAO`/`PASS_COMPOSITE` HUD timers; Task 5 validates. ✓
- TAA integration ("GTAO … require temporal accumulation") → composite is upstream of TAA, so TAA stabilizes AO noise. ✓

**Placeholder scan:** All shader bodies and Rust pass scaffolds are complete (no TBD/TODO). The two `BlurPass`/`CompositePass` structs in Task 3 Step 4 are described by interface + the canonical `GtaoPass` template they mirror, with their distinguishing bind layouts spelled out — the implementer copies the `GtaoPass` shape and adjusts bindings as listed.

**Type consistency:** `GtaoUniform` (96 B), `BlurUniform` (16 B), `CompUniform` (16 B) each have a size test. `half_res`/`half_size` return `(u32,u32)` consistently. `AO_FORMAT`/`GBUF_FORMAT`/`HDR_FORMAT` referenced by exact name. `PASS_*` renumbered as one block in Task 2 Step 5 (11 labels). TAA reads `composited_view` (Task 3 Step 5) which Task 3 Step 1 creates.

**Seams respected (from M5a/TAA final reviews):**
- All `PASS_*` consts renumbered together; `PASS_LABELS` length (11) matches `GpuTimer::new(PASS_LABELS)`.
- Jitter isolation preserved: GTAO uses the *jittered* `inv_view_proj` (matches the jittered depth), consistent with TAA; cull/shadow/sky-LUT still use the unjittered VP (untouched by this rung).
- Sky still drawn inside the main pass at depth 1.0 (now emitting a zeroed G-buffer); no extra pass.
- New shaders registered for hot-reload + given `swap_shader`, matching the taa/post pattern.
- Resize recreates every new target and rebuilds every new bind group (gtao/blur/composite), plus TAA picks up the new `composited_view`.
