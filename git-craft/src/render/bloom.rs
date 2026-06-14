use crate::render::targets::{RenderTargets, HDR_FORMAT};
use crate::render::timestamps::GpuTimer;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BloomUniform {
    pub texel: [f32; 2],
    pub karis: f32,
    pub intensity: f32,
}

/// Dynamic-offset slot per draw (universal uniform alignment).
const SLOT: u64 = 256;
/// Max draws: 6 down + 5 up.
const MAX_SLOTS: u64 = 16;

pub struct BloomPass {
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    /// Bind group per source view: [0] = HDR, [i>=1] = bloom mip i-1.
    src_groups: Vec<wgpu::BindGroup>,
    mip_count: usize,
}

impl BloomPass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &RenderTargets,
        shader_source: &str,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bloom uniforms"),
            size: SLOT * MAX_SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (down, up) = Self::build_pipelines(device, &layout, shader_source);
        let mut pass = Self {
            down,
            up,
            layout,
            sampler,
            uniform_buffer,
            src_groups: Vec::new(),
            mip_count: 0,
        };
        pass.set_targets(device, queue, targets);
        pass
    }

    fn build_pipelines(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        let make = |entry: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
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
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        (make("fs_down", wgpu::BlendState::REPLACE), make("fs_up", additive))
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        let (down, up) = Self::build_pipelines(device, &self.layout, shader_source);
        self.down = down;
        self.up = up;
    }

    /// Rebuild bind groups + per-draw uniforms after target (re)creation.
    pub fn set_targets(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &RenderTargets,
    ) {
        // Source [0] is the TAA-resolved texture (stable, not the jittered raw HDR).
        let mut views: Vec<&wgpu::TextureView> = vec![&targets.resolved_view];
        views.extend(targets.bloom_views.iter());
        self.src_groups = views
            .iter()
            .map(|view| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bloom src"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.uniform_buffer,
                                offset: 0,
                                size: Some(
                                    std::num::NonZeroU64::new(
                                        std::mem::size_of::<BloomUniform>() as u64,
                                    )
                                    .unwrap(),
                                ),
                            }),
                        },
                    ],
                })
            })
            .collect();
        self.mip_count = targets.bloom_views.len();

        // Per-draw uniforms. Down pass i samples [HDR, mip0, ..]: source size
        // is full-res for i == 0, else mip i-1. Up pass j samples mip j+1.
        let full = (targets.width, targets.height);
        for i in 0..self.mip_count {
            let src = if i == 0 { full } else { targets.bloom_sizes[i - 1] };
            let u = BloomUniform {
                texel: [1.0 / src.0 as f32, 1.0 / src.1 as f32],
                karis: if i == 0 { 1.0 } else { 0.0 },
                intensity: 1.0,
            };
            queue.write_buffer(&self.uniform_buffer, i as u64 * SLOT, bytemuck::bytes_of(&u));
        }
        for j in 0..self.mip_count.saturating_sub(1) {
            let src = targets.bloom_sizes[j + 1];
            let u = BloomUniform {
                texel: [1.0 / src.0 as f32, 1.0 / src.1 as f32],
                karis: 0.0,
                intensity: 1.0,
            };
            queue.write_buffer(
                &self.uniform_buffer,
                (self.mip_count + j) as u64 * SLOT,
                bytemuck::bytes_of(&u),
            );
        }
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets,
        timer: Option<&GpuTimer>,
        pass_slot: usize,
    ) {
        let n = self.mip_count;
        let do_draw = |dst: &wgpu::TextureView,
                       src_group: usize,
                       slot: u64,
                       pipeline: &wgpu::RenderPipeline,
                       load: wgpu::LoadOp<wgpu::Color>,
                       ts: Option<wgpu::RenderPassTimestampWrites<'_>>,
                       encoder: &mut wgpu::CommandEncoder| {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: ts,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, &self.src_groups[src_group], &[(slot * SLOT) as u32]);
            rpass.draw(0..3, 0..1);
        };
        // Timer slot brackets the whole chain: begin on the first down pass,
        // end on the last up pass. If n == 1 (tiny window, no up passes) the
        // end query is never written; pass_millis guards that with a stale
        // HUD value, so it stays a display-only quirk, never a hang.
        for i in 0..n {
            let ts = if i == 0 {
                timer.and_then(|t| t.render_writes_begin(pass_slot))
            } else {
                None
            };
            do_draw(
                &targets.bloom_views[i],
                i, // [0]=HDR, [i]=mip i-1
                i as u64,
                &self.down,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                ts,
                encoder,
            );
        }
        for j in (0..n.saturating_sub(1)).rev() {
            let ts = if j == 0 {
                timer.and_then(|t| t.render_writes_end(pass_slot))
            } else {
                None
            };
            do_draw(
                &targets.bloom_views[j],
                j + 2, // mip j+1
                (n + j) as u64,
                &self.up,
                wgpu::LoadOp::Load, // additive onto the downsampled content
                ts,
                encoder,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloom_uniform_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<BloomUniform>(), 16);
        assert_eq!(std::mem::offset_of!(BloomUniform, karis), 8);
    }
}
