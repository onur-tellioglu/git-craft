//! Temporal anti-aliasing: sub-pixel jitter sequence and the resolve pass.

use crate::render::targets::RenderTargets;
use crate::render::targets::HDR_FORMAT;

/// Halton(base) value for index i (i >= 1). Radical-inverse digit expansion.
fn halton(mut i: u32, base: u32) -> f32 {
    let mut f = 1.0;
    let mut r = 0.0;
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// Sub-pixel jitter offset in pixels, in [-0.5, 0.5], for frame `n`.
/// Halton(2,3) recentred to zero so the sequence has no DC bias.
pub fn jitter_offset(frame: u64) -> (f32, f32) {
    let i = (frame % JITTER_PERIOD) as u32 + 1;
    (halton(i, 2) - 0.5, halton(i, 3) - 0.5)
}

/// Length of the jitter cycle. 8 gives a good 1-frame-per-subsample spread
/// without a long convergence tail.
pub const JITTER_PERIOD: u64 = 8;

/// Blend weight: current frame contribution. 0.1 = 10% current, 90% history.
pub const BLEND: f32 = 0.1;

/// TAA uniform passed to the resolve shader each frame.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TaaUniform {
    /// Reconstruct world pos from this frame's depth (jittered inverse VP).
    pub inv_view_proj: [[f32; 4]; 4],
    /// Reproject world pos to last frame's screen (previous UNJITTERED VP).
    pub prev_view_proj: [[f32; 4]; 4],
    /// xy = viewport px; z = blend weight (current contribution); w = valid
    /// history flag (0 on the first frame / after resize).
    pub params: [f32; 4],
}

/// Fullscreen TAA resolve pass. Task 2: passthrough (output = current).
/// Task 3 fills in reprojection + neighborhood clamp + blend.
///
/// Writes to `targets.resolved_view`. Downstream (bloom/exposure/post) read it.
pub struct TaaPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Two bind groups: [i] reads history_views[i] as history. Ping-pong selects which to use.
    bind_groups: [wgpu::BindGroup; 2],
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
}

impl TaaPass {
    pub fn new(
        device: &wgpu::Device,
        targets: &RenderTargets,
        depth_view: &wgpu::TextureView,
        shader_source: &str,
    ) -> Self {
        let layout = Self::build_layout(device);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("taa sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa uniform"),
            size: std::mem::size_of::<TaaUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg0 = Self::build_bind_group(
            device,
            &layout,
            &targets.hdr_view,
            &targets.history_views[0],
            depth_view,
            &sampler,
            &uniform_buffer,
        );
        let bg1 = Self::build_bind_group(
            device,
            &layout,
            &targets.hdr_view,
            &targets.history_views[1],
            depth_view,
            &sampler,
            &uniform_buffer,
        );
        let pipeline = Self::build_pipeline(device, &layout, shader_source);
        Self { pipeline, layout, bind_groups: [bg0, bg1], sampler, uniform_buffer }
    }

    fn build_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("taa bind group layout"),
            entries: &[
                // binding 0: current HDR frame (filtering)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // binding 1: history texture (filtering, bilinear for reprojection)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // binding 2: depth texture — read via textureLoad (non-filtering)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // binding 3: linear-clamp sampler (for current + history)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // binding 4: TAA uniform buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        current: &wgpu::TextureView,
        history: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("taa bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(current),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(history),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn build_pipeline(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("taa shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("taa pipeline layout"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("taa pipeline"),
            layout: Some(&pipeline_layout),
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
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Write the TAA uniform for this frame.
    pub fn prepare(&self, queue: &wgpu::Queue, uniform: &TaaUniform) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniform));
    }

    /// Rebuild both bind groups after targets/depth are recreated on resize.
    pub fn rebuild_bind_groups(
        &mut self,
        device: &wgpu::Device,
        targets: &RenderTargets,
        depth_view: &wgpu::TextureView,
    ) {
        self.bind_groups = [
            Self::build_bind_group(
                device,
                &self.layout,
                &targets.hdr_view,
                &targets.history_views[0],
                depth_view,
                &self.sampler,
                &self.uniform_buffer,
            ),
            Self::build_bind_group(
                device,
                &self.layout,
                &targets.hdr_view,
                &targets.history_views[1],
                depth_view,
                &self.sampler,
                &self.uniform_buffer,
            ),
        ];
    }

    /// Hot-reload the shader.
    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.pipeline = Self::build_pipeline(device, &self.layout, shader_source);
    }

    /// Encode the TAA resolve pass, then copy resolved → history write slot.
    ///
    /// `history_read_idx`: the history_views index sampled this frame (0 or 1).
    /// Writes to `targets.resolved_view`. Copies resolved → `targets.history_textures[1-history_read_idx]`.
    /// After this call, the app should flip `taa_history_idx ^= 1`.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets,
        history_read_idx: usize,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("taa"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.resolved_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(&self.pipeline);
            // bind_groups[read_idx] has history_views[read_idx] at binding 1.
            rpass.set_bind_group(0, &self.bind_groups[history_read_idx], &[]);
            rpass.draw(0..3, 0..1);
        }

        // Copy resolved → history write slot for next frame's read.
        let write_idx = 1 - history_read_idx;
        let size = wgpu::Extent3d {
            width: targets.width.max(1),
            height: targets.height.max(1),
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.resolved_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &targets.history_textures[write_idx],
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            size,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halton_matches_known_values() {
        assert!((halton(1, 2) - 0.5).abs() < 1e-6);
        assert!((halton(2, 2) - 0.25).abs() < 1e-6);
        assert!((halton(3, 2) - 0.75).abs() < 1e-6);
        assert!((halton(1, 3) - 1.0 / 3.0).abs() < 1e-6);
        assert!((halton(2, 3) - 2.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn offsets_stay_in_half_pixel_and_recentre() {
        let mut sx = 0.0;
        let mut sy = 0.0;
        for f in 0..JITTER_PERIOD {
            let (x, y) = jitter_offset(f);
            assert!((-0.5..=0.5).contains(&x) && (-0.5..=0.5).contains(&y), "f={f}: {x},{y}");
            sx += x;
            sy += y;
        }
        // Recentred sequence has near-zero mean over a period (no DC drift).
        assert!(sx.abs() / (JITTER_PERIOD as f32) < 0.2, "mean x {sx}");
        assert!(sy.abs() / (JITTER_PERIOD as f32) < 0.2, "mean y {sy}");
    }

    #[test]
    fn taa_uniform_layout() {
        assert_eq!(std::mem::size_of::<TaaUniform>(), 144);
        assert_eq!(std::mem::offset_of!(TaaUniform, prev_view_proj), 64);
        assert_eq!(std::mem::offset_of!(TaaUniform, params), 128);
    }
}
