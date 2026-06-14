//! Cascaded Shadow Maps (CSM): cascade split/light-matrix math (pure, TDD)
//! and the GPU depth-pass renderer that renders three 1536² cascade layers
//! with texel-snapped orthographic matrices and a cadenced update schedule.

use glam::{Mat4, Vec3};

pub const CASCADE_COUNT: usize = 3;
pub const SHADOW_RESOLUTION: u32 = 1536;
/// View distance covered by the cascades. Beyond it the flood-fill skylight
/// guard takes over as the only darkening term (spec §6).
pub const SHADOW_FAR: f32 = 360.0;
/// Light-space depth margin behind/before the slice sphere so casters outside
/// the camera frustum (mountains, trees up-sun) still shadow it. World height
/// is 256, so 300 covers any caster the world can contain.
const Z_MARGIN: f32 = 300.0;

/// Practical split scheme: per-boundary blend of uniform and logarithmic.
pub fn cascade_splits(near: f32, far: f32, lambda: f32) -> [f32; CASCADE_COUNT + 1] {
    let mut s = [0.0; CASCADE_COUNT + 1];
    for (i, v) in s.iter_mut().enumerate() {
        let t = i as f32 / CASCADE_COUNT as f32;
        let uniform = near + (far - near) * t;
        // Guard near == 0 (log split undefined); uniform-only there.
        let log = if near > 0.0 {
            near * (far / near).powf(t)
        } else {
            uniform
        };
        *v = uniform * (1.0 - lambda) + log * lambda;
    }
    s
}

/// World-space corners of the camera frustum slice [near_d, far_d].
/// Order: near (-x-y, +x-y, +x+y, -x+y), then far likewise.
pub fn slice_corners(
    pos: Vec3,
    forward: Vec3,
    fov_y: f32,
    aspect: f32,
    near_d: f32,
    far_d: f32,
) -> [Vec3; 8] {
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let tan_half = (fov_y * 0.5).tan();
    let mut out = [Vec3::ZERO; 8];
    for (half, &d) in [near_d, far_d].iter().enumerate() {
        let hh = tan_half * d;
        let hw = hh * aspect;
        let c = pos + forward * d;
        out[half * 4] = c - right * hw - up * hh;
        out[half * 4 + 1] = c + right * hw - up * hh;
        out[half * 4 + 2] = c + right * hw + up * hh;
        out[half * 4 + 3] = c - right * hw + up * hh;
    }
    out
}

#[derive(Clone, Copy)]
pub struct CascadeFit {
    pub view_proj: Mat4,
    /// World size of one shadow-map texel (normal-offset bias scale).
    pub texel_world: f32,
}

/// Orthographic light matrix around the slice's bounding sphere, with the XY
/// translation snapped to whole shadow-map texels.
pub fn fit_light_matrix(corners: &[Vec3; 8], light_dir: Vec3, resolution: u32) -> CascadeFit {
    let center = corners.iter().copied().sum::<Vec3>() / 8.0;
    let radius = corners
        .iter()
        .map(|c| (*c - center).length())
        .fold(0.0f32, f32::max)
        .max(1.0);
    // Light "up" is +Z: sun/moon directions always carry a fixed ±0.119 Z
    // tilt (DayCycle::sun_dir), so they are never parallel to Z.
    let eye = center + light_dir * (radius + Z_MARGIN);
    let view = Mat4::look_to_rh(eye, -light_dir, Vec3::Z);
    let depth = 2.0 * (radius + Z_MARGIN);
    let proj = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, depth);
    let vp = proj * view;
    // Snap: move the projection so the world origin lands on a texel corner;
    // every world point then lands on the same sub-texel phase each frame.
    let half_res = resolution as f32 / 2.0;
    let origin = vp.project_point3(Vec3::ZERO);
    let snap = Mat4::from_translation(Vec3::new(
        ((origin.x * half_res).round() - origin.x * half_res) / half_res,
        ((origin.y * half_res).round() - origin.y * half_res) / half_res,
        0.0,
    ));
    CascadeFit {
        view_proj: snap * vp,
        texel_world: 2.0 * radius / resolution as f32,
    }
}

/// Update cadence (spec §6: far cascades every 2–4 frames).
pub fn cascade_due(frame: u64, cascade: usize) -> bool {
    match cascade {
        // Near and mid cascades refit every frame: cadencing them makes
        // their shadows lag-and-pop visibly while the camera moves. Only the
        // far cascade (small on screen, expensive to redraw) is cadenced, and
        // even it every other frame to keep the pop subtle.
        0 | 1 => true,
        _ => frame.is_multiple_of(2),
    }
}

// ── GPU renderer ─────────────────────────────────────────────────────────────

use crate::render::depth::DEPTH_FORMAT;
use crate::render::frustum::Frustum;
use crate::render::terrain::{MAX_SECTIONS, TerrainRenderer};
use crate::render::timestamps::GpuTimer;

/// Terrain-facing uniform: sampled by terrain.wgsl's fragment stage (Task 5).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowUniform {
    pub mats: [[[f32; 4]; 4]; CASCADE_COUNT],
    /// xyz = cascade far view-distances; w unused.
    pub splits: [f32; 4],
    /// xyz = world texel size per cascade (normal-offset scale); w unused.
    pub texels: [f32; 4],
}

const CASCADE_SLOT: u64 = 256; // dynamic-offset alignment
const INDIRECT_STRIDE: u64 = MAX_SECTIONS as u64 * 20;

pub struct ShadowRenderer {
    pipeline: wgpu::RenderPipeline,
    cascade_layout: wgpu::BindGroupLayout,
    cascade_buffer: wgpu::Buffer,
    cascade_bind_group: wgpu::BindGroup,
    layer_views: Vec<wgpu::TextureView>,
    array_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    indirect_buffer: wgpu::Buffer,
    draw_counts: [u32; CASCADE_COUNT],
    fits: [CascadeFit; CASCADE_COUNT],
    due: [bool; CASCADE_COUNT],
    frame: u64,
}

impl ShadowRenderer {
    pub fn new(
        device: &wgpu::Device,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> Self {
        // Cascade bind group layout: one uniform entry with dynamic offset.
        let cascade_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow cascade"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        // cascade_buffer: 3 slots × 256 bytes, one mat4 (64 bytes) per slot.
        let cascade_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow cascade uniforms"),
            size: CASCADE_SLOT * CASCADE_COUNT as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let cascade_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow cascade"),
            layout: &cascade_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &cascade_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(64),
                }),
            }],
        });

        // Depth texture: 1536×1536×3 array layers.
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow maps"),
            size: wgpu::Extent3d {
                width: SHADOW_RESOLUTION,
                height: SHADOW_RESOLUTION,
                depth_or_array_layers: CASCADE_COUNT as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        // Per-layer views for depth-pass render targets.
        let layer_views: Vec<wgpu::TextureView> = (0..CASCADE_COUNT as u32)
            .map(|i| {
                shadow_tex.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("shadow layer"),
                    format: Some(DEPTH_FORMAT),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    aspect: wgpu::TextureAspect::All,
                    base_mip_level: 0,
                    mip_level_count: Some(1),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    usage: None,
                })
            })
            .collect();

        // Full array view for sampling in Task 5.
        let array_view = shadow_tex.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow array"),
            format: Some(DEPTH_FORMAT),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(CASCADE_COUNT as u32),
            usage: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow uniform"),
            size: std::mem::size_of::<ShadowUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let indirect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow indirect"),
            size: INDIRECT_STRIDE * CASCADE_COUNT as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline = Self::build_pipeline(device, &cascade_layout, quads_layout, shader_source);

        let fits = std::array::from_fn(|_| CascadeFit {
            view_proj: Mat4::IDENTITY,
            texel_world: 1.0,
        });

        Self {
            pipeline,
            cascade_layout,
            cascade_buffer,
            cascade_bind_group,
            layer_views,
            array_view,
            uniform_buffer,
            indirect_buffer,
            draw_counts: [0; CASCADE_COUNT],
            fits,
            due: [false; CASCADE_COUNT],
            frame: 0,
        }
    }

    fn build_pipeline(
        device: &wgpu::Device,
        cascade_layout: &wgpu::BindGroupLayout,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(cascade_layout), Some(quads_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    // Slope-scaled against acne on grazing-lit faces (the main
                    // source of self-shadow flicker under camera motion). The
                    // normal-offset in terrain.wgsl handles the rest.
                    constant: 3,
                    slope_scale: 3.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn swap_shader(
        &mut self,
        device: &wgpu::Device,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) {
        self.pipeline =
            Self::build_pipeline(device, &self.cascade_layout, quads_layout, shader_source);
    }

    /// Returns the uniform buffer sampled by terrain.wgsl's fragment stage.
    pub fn uniform_buffer(&self) -> &wgpu::Buffer {
        &self.uniform_buffer
    }

    /// Returns the D2Array view of all cascade depth maps, sampled by terrain.
    pub fn array_view(&self) -> &wgpu::TextureView {
        &self.array_view
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        terrain: &TerrainRenderer,
        cam_pos: Vec3,
        cam_forward: Vec3,
        fov_y: f32,
        aspect: f32,
        light_dir: Vec3,
    ) {
        self.frame += 1;
        let splits = cascade_splits(0.5, SHADOW_FAR, 0.7);

        for i in 0..CASCADE_COUNT {
            if self.frame != 1 && !cascade_due(self.frame, i) {
                continue;
            }
            let corners = slice_corners(
                cam_pos,
                cam_forward,
                fov_y,
                aspect,
                splits[i],
                splits[i + 1],
            );
            let fit = fit_light_matrix(&corners, light_dir, SHADOW_RESOLUTION);
            // Write the mat4 for this cascade slot into cascade_buffer.
            let mat_cols = fit.view_proj.to_cols_array();
            queue.write_buffer(
                &self.cascade_buffer,
                i as u64 * CASCADE_SLOT,
                bytemuck::cast_slice(&mat_cols),
            );
            let frustum = Frustum::from_view_proj(fit.view_proj);
            self.draw_counts[i] = terrain.write_indirect_for(
                queue,
                &frustum,
                &self.indirect_buffer,
                i as u64 * INDIRECT_STRIDE,
            );
            self.fits[i] = fit;
            self.due[i] = true;
        }

        // Always write the full ShadowUniform from cached fits.
        let uniform = ShadowUniform {
            mats: std::array::from_fn(|i| self.fits[i].view_proj.to_cols_array_2d()),
            splits: [splits[1], splits[2], splits[3], 0.0],
            texels: [
                self.fits[0].texel_world,
                self.fits[1].texel_world,
                self.fits[2].texel_world,
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        terrain: &TerrainRenderer,
        timer: Option<&GpuTimer>,
        first_pass_slot: usize,
    ) {
        for i in 0..CASCADE_COUNT {
            if !std::mem::take(&mut self.due[i]) {
                continue;
            }
            let ts_writes = timer.and_then(|t| t.render_writes(first_pass_slot + i));
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.layer_views[i],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: ts_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(
                0,
                &self.cascade_bind_group,
                &[(i as u64 * CASCADE_SLOT) as u32],
            );
            rpass.set_bind_group(1, terrain.quads_bind_group(), &[]);
            rpass.set_index_buffer(terrain.index_buffer().slice(..), wgpu::IndexFormat::Uint32);
            for d in 0..self.draw_counts[i] {
                rpass.draw_indexed_indirect(
                    &self.indirect_buffer,
                    i as u64 * INDIRECT_STRIDE + d as u64 * 20,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: u32 = SHADOW_RESOLUTION;

    fn light() -> Vec3 {
        Vec3::new(1.0, 1.0, 0.12).normalize()
    }

    #[test]
    fn splits_cover_the_range_monotonically() {
        let s = cascade_splits(0.5, SHADOW_FAR, 0.7);
        assert_eq!(s[0], 0.5);
        assert_eq!(s[CASCADE_COUNT], SHADOW_FAR);
        for i in 0..CASCADE_COUNT {
            assert!(s[i] < s[i + 1], "splits not increasing: {s:?}");
        }
    }

    #[test]
    fn lambda_zero_gives_uniform_splits() {
        let s = cascade_splits(0.0, 300.0, 0.0);
        assert!(
            (s[1] - 100.0).abs() < 1e-3 && (s[2] - 200.0).abs() < 1e-3,
            "{s:?}"
        );
    }

    #[test]
    fn slice_corners_match_hand_computed_frustum() {
        // fov 90° (tan = 1), aspect 1, looking down -Z from the origin:
        // near plane at 10 has corners (±10, ±10, -10).
        let c = slice_corners(Vec3::ZERO, Vec3::NEG_Z, 90f32.to_radians(), 1.0, 10.0, 20.0);
        let expect_near = [
            Vec3::new(-10.0, -10.0, -10.0),
            Vec3::new(10.0, -10.0, -10.0),
            Vec3::new(10.0, 10.0, -10.0),
            Vec3::new(-10.0, 10.0, -10.0),
        ];
        for (got, want) in c[..4].iter().zip(expect_near) {
            assert!((*got - want).length() < 1e-3, "{got} != {want}");
        }
        assert!(
            (c[4] - Vec3::new(-20.0, -20.0, -20.0)).length() < 1e-3,
            "far corner: {}",
            c[4]
        );
    }

    #[test]
    fn light_matrix_contains_every_slice_corner() {
        let corners = slice_corners(
            Vec3::new(100.0, 80.0, -40.0),
            Vec3::new(0.6, -0.3, 0.74).normalize(),
            70f32.to_radians(),
            1.6,
            32.0,
            128.0,
        );
        let fit = fit_light_matrix(&corners, light(), RES);
        for c in corners {
            let ndc = fit.view_proj.project_point3(c);
            assert!(
                ndc.x.abs() <= 1.001 && ndc.y.abs() <= 1.001,
                "corner outside XY: {ndc}"
            );
            assert!((0.0..=1.0).contains(&ndc.z), "corner outside depth: {ndc}");
        }
    }

    #[test]
    fn texel_snap_quantizes_the_world_origin() {
        // Two slightly different camera positions must produce light matrices
        // whose texel grids coincide: the world origin always projects onto
        // an integer texel coordinate.
        for dx in [0.0, 0.013, 1.77] {
            let corners = slice_corners(
                Vec3::new(50.0 + dx, 70.0, 10.0),
                Vec3::NEG_Z,
                70f32.to_radians(),
                1.6,
                0.5,
                32.0,
            );
            let fit = fit_light_matrix(&corners, light(), RES);
            let t = fit.view_proj.project_point3(Vec3::ZERO);
            let tx = t.x * RES as f32 / 2.0;
            let ty = t.y * RES as f32 / 2.0;
            assert!(
                (tx - tx.round()).abs() < 1e-3,
                "X off-grid by {} texels",
                tx - tx.round()
            );
            assert!(
                (ty - ty.round()).abs() < 1e-3,
                "Y off-grid by {} texels",
                ty - ty.round()
            );
        }
    }

    #[test]
    fn texel_world_size_matches_the_ortho_diameter() {
        let corners = slice_corners(Vec3::ZERO, Vec3::NEG_Z, 70f32.to_radians(), 1.6, 0.5, 32.0);
        let center = corners.iter().copied().sum::<Vec3>() / 8.0;
        let radius = corners
            .iter()
            .map(|c| (*c - center).length())
            .fold(0.0f32, f32::max);
        let fit = fit_light_matrix(&corners, light(), RES);
        assert!((fit.texel_world - 2.0 * radius / RES as f32).abs() < 1e-5);
    }

    #[test]
    fn cascade_cadence_matches_the_spec() {
        // Near + mid cascades refit every frame (no lag-and-pop); the far
        // cascade refits every other frame.
        for f in 1..=8u64 {
            assert!(cascade_due(f, 0));
            assert!(cascade_due(f, 1));
        }
        assert!(cascade_due(2, 2) && !cascade_due(3, 2) && cascade_due(4, 2) && !cascade_due(5, 2));
    }

    #[test]
    fn shadow_uniform_layout_matches_wgsl() {
        // 3 mat4 (192) + splits vec4 (16) + texels vec4 (16).
        assert_eq!(std::mem::size_of::<ShadowUniform>(), 224);
        assert_eq!(std::mem::offset_of!(ShadowUniform, splits), 192);
        assert_eq!(std::mem::offset_of!(ShadowUniform, texels), 208);
    }
}
