use glam::IVec3;

use crate::render::depth::DEPTH_FORMAT;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OutlineUniform {
    view_proj: [[f32; 4]; 4],
    block: [f32; 4],
}

/// Wireframe outline for the targeted block: one 24-vertex LineList draw,
/// vertices pulled from a const table in the shader.
pub struct OutlineRenderer {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    target: Option<IVec3>,
}

impl OutlineRenderer {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        shader_source: &str,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("outline"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("outline"),
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
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("outline"),
            size: std::mem::size_of::<OutlineUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("outline"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("outline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("outline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[], // vertex pulling: corner table lives in the shader
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                // Read-only depth: lines sit ON the block faces; LessEqual
                // keeps them visible without disturbing the depth buffer.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: crate::render::targets::GBUF_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, buffer, bind_group, target: None }
    }

    /// Update the targeted block (None hides the outline). Call before the
    /// render pass; write_buffer lands before the subsequent submit.
    pub fn set_target(
        &mut self,
        queue: &wgpu::Queue,
        view_proj: glam::Mat4,
        target: Option<IVec3>,
    ) {
        self.target = target;
        if let Some(t) = target {
            let uniform = OutlineUniform {
                view_proj: view_proj.to_cols_array_2d(),
                block: [t.x as f32, t.y as f32, t.z as f32, 0.0],
            };
            queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&uniform));
        }
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass<'_>) {
        if self.target.is_none() {
            return;
        }
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..24, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shipped_outline_shader_is_valid() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/outline.wgsl"
        ))
        .unwrap();
        assert!(crate::render::hot_reload::validate_wgsl(&src).is_ok());
    }
}
