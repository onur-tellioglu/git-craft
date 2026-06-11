use wgpu::util::DeviceExt;

use crate::mesh::quad::{build_quad_indices, PackedQuad};
use crate::render::depth::DEPTH_FORMAT;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

pub struct TerrainRenderer {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_layout: wgpu::BindGroupLayout,
    quads_layout: wgpu::BindGroupLayout,
    quads_bind_group: Option<wgpu::BindGroup>,
    // Kept alive explicitly: the bind group referencing it must never outlive it.
    quads_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    index_count: u32,
    surface_format: wgpu::TextureFormat,
}

impl TerrainRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat, shader_source: &str) -> Self {
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let quads_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quads"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });

        let pipeline = Self::build_pipeline(device, surface_format, &camera_layout, &quads_layout, shader_source);

        Self {
            pipeline,
            camera_buffer,
            camera_bind_group,
            camera_layout,
            quads_layout,
            quads_bind_group: None,
            quads_buffer: None,
            index_buffer: None,
            index_count: 0,
            surface_format,
        }
    }

    fn build_pipeline(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(camera_layout), Some(quads_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[], // vertex pulling: no vertex buffers
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None, // M2 establishes winding + backface culling
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Replace the pipeline with one built from new shader source (hot-reload, Task 9).
    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.pipeline = Self::build_pipeline(
            device, self.surface_format, &self.camera_layout, &self.quads_layout, shader_source,
        );
    }

    pub fn upload_quads(&mut self, device: &wgpu::Device, quads: &[PackedQuad]) {
        if quads.is_empty() {
            self.quads_bind_group = None;
            self.quads_buffer = None;
            self.index_buffer = None;
            self.index_count = 0;
            return;
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quads"),
            contents: bytemuck::cast_slice(quads),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.quads_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quads"),
            layout: &self.quads_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        }));
        self.quads_buffer = Some(buffer);
        let indices = build_quad_indices(quads.len() as u32);
        self.index_buffer = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        }));
        self.index_count = indices.len() as u32;
    }

    pub fn write_camera(&self, queue: &wgpu::Queue, view_proj: glam::Mat4) {
        let uniform = CameraUniform { view_proj: view_proj.to_cols_array_2d() };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass<'_>) {
        let (Some(quads_bg), Some(index_buffer)) = (&self.quads_bind_group, &self.index_buffer) else {
            return;
        };
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.camera_bind_group, &[]);
        rpass.set_bind_group(1, quads_bg, &[]);
        rpass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
