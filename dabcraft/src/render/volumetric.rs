//! Froxel volumetrics (M5c): god rays + height fog.
//!
//! Two compute passes build a fixed view-frustum 3D grid (owned here, like the
//! sky LUTs — screen-independent, so resize never touches it). `cs_inscatter`
//! samples the CSM for the directional shaft term; `cs_integrate` accumulates
//! front-to-back. The composite pass (gtao.rs) samples the integrated grid.
//!
//! Pure helpers (slice↔view-distance mapping, HG phase) are unit-tested; the
//! rendered result is validated via the F3 HUD timers, per project convention.

/// Froxel grid resolution. Mirrors the `VOL_*` consts in volumetric.wgsl.
pub const VOL_W: u32 = 160;
pub const VOL_H: u32 = 90;
pub const VOL_D: u32 = 64;
/// Near/far view distances the grid spans, in world meters (1 block = 1 m).
/// VOL_FAR matches shadow::SHADOW_FAR — beyond the cascades the shaft term is
/// 1.0 anyway, so there is no point marching god rays farther.
pub const VOL_NEAR: f32 = 0.5;
pub const VOL_FAR: f32 = 360.0;

const FROXEL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// GPU-side uniform mirroring the WGSL `VolUniform` (volumetric.wgsl).
/// Layout: 2×mat4 (128) + 6×vec4 (96) = 224 B.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VolUniform {
    pub inv_view_proj: [[f32; 4]; 4],
    pub prev_view_proj: [[f32; 4]; 4],
    /// xyz world camera pos, w frame index (drives the per-froxel depth jitter).
    pub camera: [f32; 4],
    /// xyz light dir (toward the sun/moon), w 1=sun 0=moon.
    pub sun: [f32; 4],
    /// rgb directional light radiance.
    pub sun_color: [f32; 4],
    /// rgb ambient sky color (linear), w history-valid (M5c Task 3).
    pub sky: [f32; 4],
    /// x density, y haze floor, z fog_y0, w fog_h.
    pub fog: [f32; 4],
    /// x absorb, y hg_g, z ambient strength, w taa_alpha.
    pub tune: [f32; 4],
}

// The froxel slice↔view-distance mapping is evaluated on the GPU (volumetric.wgsl
// builds the grid, composite.wgsl samples it). These Rust mirrors exist only to
// unit-test that the exponential distribution behaves as intended.

/// Front edge of slice `z` (fractional allowed), in world meters.
#[cfg(test)]
fn slice_to_view_dist(z: f32) -> f32 {
    VOL_NEAR * (VOL_FAR / VOL_NEAR).powf(z / VOL_D as f32)
}

/// Inverse of `slice_to_view_dist`: the (fractional) slice a view distance maps to.
#[cfg(test)]
fn view_dist_to_slice(d: f32) -> f32 {
    VOL_D as f32 * (d.max(VOL_NEAR) / VOL_NEAR).ln() / (VOL_FAR / VOL_NEAR).ln()
}

/// Henyey-Greenstein phase (test hook mirroring the WGSL).
#[cfg(test)]
fn hg_phase(cos_t: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = (1.0 + g2 - 2.0 * g * cos_t).max(1e-4);
    (1.0 - g2) / (4.0 * std::f32::consts::PI * denom * denom.sqrt())
}

fn froxel_texture(device: &wgpu::Device, label: &str) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: VOL_W, height: VOL_H, depth_or_array_layers: VOL_D },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: FROXEL_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

struct PassUnit {
    pipeline: wgpu::ComputePipeline,
    in_group: wgpu::BindGroup,
    out_group: wgpu::BindGroup,
    workgroups: (u32, u32, u32),
}

pub struct VolumetricPass {
    uniform: wgpu::Buffer,
    scatter_view: wgpu::TextureView,
    integrated_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    // Cloned (Arc-backed) shadow handles so swap_shader can rebuild without the
    // caller re-threading them; both are fixed-size and stable across resize.
    shadow_uniform: wgpu::Buffer,
    shadow_array: wgpu::TextureView,
    passes: Vec<PassUnit>, // [inscatter, integrate]
}

impl VolumetricPass {
    pub fn new(
        device: &wgpu::Device,
        shadow_uniform: &wgpu::Buffer,
        shadow_array: &wgpu::TextureView,
        shader_source: &str,
    ) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vol uniform"),
            size: std::mem::size_of::<VolUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scatter_view = froxel_texture(device, "froxel scatter");
        let integrated_view = froxel_texture(device, "froxel integrated");
        // Comparison sampler for the CSM (mirrors terrain's shadow sampler).
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vol shadow compare"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let mut pass = Self {
            uniform,
            scatter_view,
            integrated_view,
            sampler,
            shadow_uniform: shadow_uniform.clone(),
            shadow_array: shadow_array.clone(),
            passes: Vec::new(),
        };
        pass.build_passes(device, shader_source);
        pass
    }

    /// The integrated froxel grid (in-scatter rgb, transmittance a) — read by
    /// the composite pass to apply `color·a + rgb`.
    pub fn integrated_view(&self) -> &wgpu::TextureView {
        &self.integrated_view
    }

    fn build_passes(&mut self, device: &wgpu::Device, shader_source: &str) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("volumetric"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // --- cs_inscatter: group0 = vol/shadow/CSM, group1 = out_scatter(0).
        let in_layout_is = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vol inscatter in"),
            entries: &[
                uniform_entry(0),
                uniform_entry(1),
                depth_array_entry(2),
                comparison_sampler_entry(3),
            ],
        });
        let out_layout_is = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vol inscatter out"),
            entries: &[storage3d_entry(0)],
        });
        let in_group_is = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vol inscatter in"),
            layout: &in_layout_is,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.shadow_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.shadow_array),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let out_group_is = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vol inscatter out"),
            layout: &out_layout_is,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&self.scatter_view),
            }],
        });
        let layout_is = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vol inscatter"),
            bind_group_layouts: &[Some(&in_layout_is), Some(&out_layout_is)],
            immediate_size: 0,
        });
        let pipeline_is = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vol inscatter"),
            layout: Some(&layout_is),
            module: &shader,
            entry_point: Some("cs_inscatter"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // --- cs_integrate: group0 = in_scatter(4), group1 = out_integrated(1).
        let in_layout_in = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vol integrate in"),
            entries: &[float3d_entry(4)],
        });
        let out_layout_in = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vol integrate out"),
            entries: &[storage3d_entry(1)],
        });
        let in_group_in = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vol integrate in"),
            layout: &in_layout_in,
            entries: &[wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&self.scatter_view),
            }],
        });
        let out_group_in = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vol integrate out"),
            layout: &out_layout_in,
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&self.integrated_view),
            }],
        });
        let layout_in = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vol integrate"),
            bind_group_layouts: &[Some(&in_layout_in), Some(&out_layout_in)],
            immediate_size: 0,
        });
        let pipeline_in = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vol integrate"),
            layout: Some(&layout_in),
            module: &shader,
            entry_point: Some("cs_integrate"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        self.passes = vec![
            PassUnit {
                pipeline: pipeline_is,
                in_group: in_group_is,
                out_group: out_group_is,
                workgroups: (VOL_W.div_ceil(4), VOL_H.div_ceil(4), VOL_D.div_ceil(4)),
            },
            PassUnit {
                pipeline: pipeline_in,
                in_group: in_group_in,
                out_group: out_group_in,
                workgroups: (VOL_W.div_ceil(8), VOL_H.div_ceil(8), 1),
            },
        ];
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.build_passes(device, shader_source);
    }

    pub fn prepare(&self, queue: &wgpu::Queue, u: &VolUniform) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(u));
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("volumetric"),
            timestamp_writes,
        });
        for unit in &self.passes {
            cpass.set_pipeline(&unit.pipeline);
            cpass.set_bind_group(0, &unit.in_group, &[]);
            cpass.set_bind_group(1, &unit.out_group, &[]);
            cpass.dispatch_workgroups(unit.workgroups.0, unit.workgroups.1, unit.workgroups.2);
        }
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn depth_array_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn comparison_sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
        count: None,
    }
}

fn float3d_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D3,
            multisampled: false,
        },
        count: None,
    }
}

fn storage3d_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: FROXEL_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D3,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vol_uniform_is_224_bytes() {
        assert_eq!(std::mem::size_of::<VolUniform>(), 224);
        assert_eq!(std::mem::offset_of!(VolUniform, prev_view_proj), 64);
        assert_eq!(std::mem::offset_of!(VolUniform, camera), 128);
        assert_eq!(std::mem::offset_of!(VolUniform, fog), 192);
        assert_eq!(std::mem::offset_of!(VolUniform, tune), 208);
    }

    #[test]
    fn slice_view_dist_roundtrips() {
        // Endpoints land on the grid bounds, and the mapping is invertible.
        assert!((slice_to_view_dist(0.0) - VOL_NEAR).abs() < 1e-3);
        assert!((slice_to_view_dist(VOL_D as f32) - VOL_FAR).abs() < 1e-1);
        for &d in &[1.0_f32, 8.0, 64.0, 200.0, 359.0] {
            let z = view_dist_to_slice(d);
            assert!((slice_to_view_dist(z) - d).abs() < 1e-2, "d={d} z={z}");
        }
    }

    #[test]
    fn slices_are_dense_near_then_spread() {
        // Exponential distribution: the first slice is far thinner than the last.
        let near = slice_to_view_dist(1.0) - slice_to_view_dist(0.0);
        let far = slice_to_view_dist(VOL_D as f32) - slice_to_view_dist(VOL_D as f32 - 1.0);
        assert!(far > near * 10.0, "near {near} far {far}");
    }

    #[test]
    fn hg_phase_normalizes_over_the_sphere() {
        // ∫ phase dΩ ≈ 1 (Riemann sum over cosθ ∈ [-1, 1], 2π azimuth).
        let g = 0.6;
        let n = 2000;
        let mut sum = 0.0;
        for i in 0..n {
            let cos_t = -1.0 + 2.0 * (i as f32 + 0.5) / n as f32;
            sum += hg_phase(cos_t, g);
        }
        let integral = sum * (2.0 / n as f32) * 2.0 * std::f32::consts::PI;
        assert!((integral - 1.0).abs() < 0.02, "phase integral {integral}");
    }

    #[test]
    fn forward_scattering_peaks_toward_the_sun() {
        let g = 0.6;
        assert!(hg_phase(1.0, g) > hg_phase(-1.0, g) * 10.0);
    }
}
