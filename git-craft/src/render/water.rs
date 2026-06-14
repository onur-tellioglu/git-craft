//! Transparent water (M5d). The water pipeline pulls vertices from the same
//! quad/section storage buffers as terrain (via terrain's camera + quads bind
//! groups), and adds a group-2 of water resources: the opaque scene color
//! (refraction/SSR source), the opaque depth (manual test + fog), and the
//! sky-view LUT (reflection). Drawn after `composite`, before TAA.

use crate::render::terrain::TerrainRenderer;

/// Mirrors the WGSL `WaterUniform` (water.wgsl). 32 B.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterUniform {
    /// rgb tint color, w fog density per meter of column depth.
    pub tint: [f32; 4],
    /// x time, y fresnel F0, z reflection strength, w refraction offset.
    pub params: [f32; 4],
}

pub struct WaterRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    skyview: wgpu::TextureView,
    bind_group: Option<wgpu::BindGroup>,
}

impl WaterRenderer {
    pub fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        quads_layout: &wgpu::BindGroupLayout,
        skyview: &wgpu::TextureView,
        shader_source: &str,
    ) -> Self {
        let sampled = |binding: u32, filterable: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water"),
            entries: &[
                sampled(0, true),  // scene color
                sampled(1, false), // depth (textureLoad)
                sampled(2, true),  // sky-view LUT
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
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
            label: Some("water"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water uniform"),
            size: std::mem::size_of::<WaterUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline =
            Self::build_pipeline(device, camera_layout, quads_layout, &layout, shader_source);
        Self {
            pipeline,
            layout,
            sampler,
            uniform,
            skyview: skyview.clone(),
            bind_group: None,
        }
    }

    fn build_pipeline(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        quads_layout: &wgpu::BindGroupLayout,
        water_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water"),
            bind_group_layouts: &[Some(camera_layout), Some(quads_layout), Some(water_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None, // manual depth test in the shader (sampled depth)
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

    /// (Re)build the group-2 bind group with the current scene-color + depth
    /// views (both recreated on resize / render-scale changes).
    pub fn rebuild_bind_group(
        &mut self,
        device: &wgpu::Device,
        scene_color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scene_color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.skyview),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        }));
    }

    pub fn swap_shader(
        &mut self,
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) {
        self.pipeline = Self::build_pipeline(
            device,
            camera_layout,
            quads_layout,
            &self.layout,
            shader_source,
        );
    }

    pub fn prepare(&self, queue: &wgpu::Queue, u: &WaterUniform) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(u));
    }

    /// Draw the resident water surfaces into the bound HDR target. Reuses
    /// terrain's camera + quads bind groups and the per-frame water indirect buffer.
    pub fn draw(&self, rpass: &mut wgpu::RenderPass<'_>, terrain: &TerrainRenderer) {
        let Some(bg) = &self.bind_group else { return };
        let count = terrain.water_visible_count();
        if count == 0 {
            return;
        }
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, terrain.camera_bind_group(), &[]);
        rpass.set_bind_group(1, terrain.quads_bind_group(), &[]);
        rpass.set_bind_group(2, bg, &[]);
        rpass.set_index_buffer(terrain.index_buffer().slice(..), wgpu::IndexFormat::Uint32);
        for i in 0..count {
            rpass.draw_indexed_indirect(terrain.water_indirect_buffer(), i as u64 * 20);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_uniform_is_32_bytes() {
        assert_eq!(std::mem::size_of::<WaterUniform>(), 32);
    }
}
