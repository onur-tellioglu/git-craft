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

pub struct GtaoPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
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
                // 2: uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
        let bind_group =
            Self::build_bind_group(device, &layout, depth_view, gbuf_view, &uniform);
        let pipeline = Self::build_pipeline(device, &layout, shader_source);
        Self { pipeline, layout, bind_group, uniform }
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        depth_view: &wgpu::TextureView,
        gbuf_view: &wgpu::TextureView,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(gbuf_view),
                },
                wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
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
            bind_group_layouts: &[Some(layout)],
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
            device,
            &self.layout,
            depth_view,
            gbuf_view,
            &self.uniform,
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

/// Uniform for the bilateral blur pass. 16 bytes (one vec4).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BlurUniform {
    pub params: [f32; 4], // xy half-res px, z depth sigma, w unused
}

/// Depth-aware bilateral blur of the raw half-res AO into ao_blur_view.
pub struct BlurPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl BlurPass {
    pub fn new(
        device: &wgpu::Device,
        ao_raw_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        shader_source: &str,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur bgl"),
            entries: &[
                // 0: ao_raw (non-filterable, textureLoad)
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
                // 1: depth (non-filterable, textureLoad)
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
                // 2: uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
            label: Some("blur uniform"),
            size: std::mem::size_of::<BlurUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group =
            Self::build_bind_group(device, &layout, ao_raw_view, depth_view, &uniform);
        let pipeline = Self::build_pipeline(device, &layout, shader_source);
        Self { pipeline, layout, bind_group, uniform }
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        ao_raw_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(ao_raw_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
            ],
        })
    }

    fn build_pipeline(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blur pl"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blur pipeline"),
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

    pub fn prepare(&self, queue: &wgpu::Queue, params: [f32; 4]) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&BlurUniform { params }));
    }

    pub fn rebuild_bind_group(
        &mut self,
        device: &wgpu::Device,
        ao_raw_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
    ) {
        self.bind_group =
            Self::build_bind_group(device, &self.layout, ao_raw_view, depth_view, &self.uniform);
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.pipeline = Self::build_pipeline(device, &self.layout, shader_source);
    }

    /// Render the bilateral-blurred AO into `ao_blur_view`.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        ao_blur_view: &wgpu::TextureView,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gtao_blur"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ao_blur_view,
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

/// Uniform for the composite pass. Controls debug visualization modes.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CompUniform {
    pub flags: [f32; 4], // x: 1.0 = AO debug view, yzw reserved
}

/// Composites AO onto the HDR ambient term, writing a full-res composited target.
/// TAA reads this instead of the raw hdr_view.
pub struct CompositePass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
}

impl CompositePass {
    pub fn new(
        device: &wgpu::Device,
        hdr_view: &wgpu::TextureView,
        gbuf_view: &wgpu::TextureView,
        ao_blur_view: &wgpu::TextureView,
        shader_source: &str,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite bgl"),
            entries: &[
                // 0: hdr (filterable: true — textureLoad ignores sampler, format is filterable)
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
                // 1: gbuf (filterable: true — Rgba8Unorm is filterable, textureLoad ignores sampler)
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
                // 2: ao_blur (filterable: true — used with textureSampleLevel for bilinear upsample)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 3: filtering sampler (for ao bilinear upsample; textureLoad ignores it for hdr/gbuf)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // 4: debug flags uniform
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
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("composite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite uniform"),
            size: std::mem::size_of::<CompUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = Self::build_bind_group(
            device,
            &layout,
            hdr_view,
            gbuf_view,
            ao_blur_view,
            &sampler,
            &uniform,
        );
        let pipeline = Self::build_pipeline(device, &layout, shader_source);
        Self { pipeline, layout, bind_group, sampler, uniform }
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        gbuf_view: &wgpu::TextureView,
        ao_blur_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(gbuf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(ao_blur_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry { binding: 4, resource: uniform.as_entire_binding() },
            ],
        })
    }

    fn build_pipeline(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite pl"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("composite pipeline"),
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
                    format: crate::render::targets::HDR_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn rebuild_bind_group(
        &mut self,
        device: &wgpu::Device,
        hdr_view: &wgpu::TextureView,
        gbuf_view: &wgpu::TextureView,
        ao_blur_view: &wgpu::TextureView,
    ) {
        self.bind_group = Self::build_bind_group(
            device,
            &self.layout,
            hdr_view,
            gbuf_view,
            ao_blur_view,
            &self.sampler,
            &self.uniform,
        );
    }

    /// Write the debug flag to the GPU uniform. Call once per frame before encode.
    pub fn set_debug(&self, queue: &wgpu::Queue, on: bool) {
        let u = CompUniform { flags: [if on { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0] };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.pipeline = Self::build_pipeline(device, &self.layout, shader_source);
    }

    /// Render the AO-composited HDR into `composited_view`.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        composited_view: &wgpu::TextureView,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: composited_view,
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
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
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

    #[test]
    fn blur_uniform_is_16_bytes() {
        assert_eq!(std::mem::size_of::<BlurUniform>(), 16);
    }

    #[test]
    fn comp_uniform_is_16_bytes() {
        assert_eq!(std::mem::size_of::<CompUniform>(), 16);
    }
}
