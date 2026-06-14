/// Histogram auto-exposure: a 256-bin log2-luminance compute pass over the HDR
/// target, followed by a single-workgroup resolve that adapts a smoothed
/// exposure value. The result lives in a 16-byte storage buffer read directly
/// by the post pass — no CPU readback.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ExposureUniform {
    dt: f32,
    min_log_lum: f32,
    inv_log_range: f32,
    log_range: f32,
}

pub struct ExposurePass {
    histogram: wgpu::ComputePipeline,
    resolve: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    bins: wgpu::Buffer,
    result: wgpu::Buffer,
    uniform: wgpu::Buffer,
}

impl ExposurePass {
    pub fn new(device: &wgpu::Device, hdr_view: &wgpu::TextureView, shader_source: &str) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("exposure"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        // 256 bins × 4 bytes. wgpu zero-initialises unmapped buffers, so frame 0 is clean.
        let bins = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure bins"),
            size: 256 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let result = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure result"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure uniform"),
            size: std::mem::size_of::<ExposureUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("exposure"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("exposure"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let histogram = make("cs_histogram");
        let resolve = make("cs_resolve");
        let bind_group =
            Self::build_bind_group(device, &layout, hdr_view, &bins, &result, &uniform);
        Self {
            histogram,
            resolve,
            layout,
            bind_group,
            bins,
            result,
            uniform,
        }
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        bins: &wgpu::Buffer,
        result: &wgpu::Buffer,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exposure"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: bins.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: result.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }

    /// Rebuild the bind group after the HDR texture is recreated on resize.
    pub fn set_input(&mut self, device: &wgpu::Device, hdr_view: &wgpu::TextureView) {
        self.bind_group = Self::build_bind_group(
            device,
            &self.layout,
            hdr_view,
            &self.bins,
            &self.result,
            &self.uniform,
        );
    }

    /// Rebuild the compute pipelines after a hot-reload.
    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("exposure"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("exposure"),
            bind_group_layouts: &[Some(&self.layout)],
            immediate_size: 0,
        });
        let make = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        self.histogram = make("cs_histogram");
        self.resolve = make("cs_resolve");
    }

    /// The GPU-side result buffer: post pass binds this directly (no CPU readback).
    pub fn result_buffer(&self) -> &wgpu::Buffer {
        &self.result
    }

    /// Write the per-frame uniform (dt, fixed luminance range constants).
    pub fn prepare(&self, queue: &wgpu::Queue, dt: f32) {
        let u = ExposureUniform {
            dt,
            min_log_lum: -12.0,
            inv_log_range: 1.0 / 18.0,
            log_range: 18.0,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
    }

    /// Dispatch histogram + resolve into `encoder`.
    ///
    /// The two dispatches MUST be in separate compute passes: `bins` is written
    /// by the histogram and read by the resolve, and WebGPU does not order
    /// storage writes across dispatches within one pass. The pass boundary makes
    /// wgpu insert the barrier. The histogram (full-frame) carries the timer
    /// slot; the single-workgroup resolve is too cheap to be worth its own slot.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        width: u32,
        height: u32,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("exposure histogram"),
                timestamp_writes,
            });
            cpass.set_pipeline(&self.histogram);
            cpass.set_bind_group(0, &self.bind_group, &[]);
            // Stride 2 in the shader: 16×16 groups cover (width/2, height/2) pixels.
            cpass.dispatch_workgroups(width.div_ceil(32), height.div_ceil(32), 1);
        }
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("exposure resolve"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.resolve);
            cpass.set_bind_group(0, &self.bind_group, &[]);
            cpass.dispatch_workgroups(1, 1, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_uniform_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<ExposureUniform>(), 16);
        assert_eq!(std::mem::offset_of!(ExposureUniform, min_log_lum), 4);
    }
}
