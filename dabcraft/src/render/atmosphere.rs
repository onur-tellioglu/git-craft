use glam::Vec3;

/// GPU-side uniform mirroring the WGSL `AtmUniform` struct in sky_luts.wgsl.
/// Layout: mat4 (64 B) + 3 × vec4 (48 B) = 112 B.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AtmUniform {
    pub inv_view_proj: [[f32; 4]; 4],
    /// xyz world camera pos (meters), w altitude in km.
    pub camera: [f32; 4],
    /// xyz toward the sun (world space).
    pub sun: [f32; 4],
    /// rgb top-of-atmosphere sun radiance.
    pub sun_radiance: [f32; 4],
}

const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

fn lut_texture(device: &wgpu::Device, label: &str, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LUT_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
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

fn sampled_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn storage_entry(binding: u32, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: LUT_FORMAT,
            view_dimension: dim,
        },
        count: None,
    }
}

/// One (group0, group1, pipeline) triple per LUT entry point.
struct LutPass {
    pipeline: wgpu::ComputePipeline,
    in_group: wgpu::BindGroup,
    out_group: wgpu::BindGroup,
    workgroups: (u32, u32, u32),
}

pub struct SkyLuts {
    uniform: wgpu::Buffer,
    pub skyview_view: wgpu::TextureView,
    transmittance_view: wgpu::TextureView,
    multiscatter_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    passes: Vec<LutPass>, // [transmittance, multiscatter, skyview]
    statics_done: bool,
}

impl SkyLuts {
    pub fn new(device: &wgpu::Device, shader_source: &str) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atm uniform"),
            size: std::mem::size_of::<AtmUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let transmittance_view = lut_texture(device, "transmittance lut", 256, 64);
        let multiscatter_view = lut_texture(device, "multiscatter lut", 32, 32);
        let skyview_view = lut_texture(device, "skyview lut", 192, 108);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lut"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let mut luts = Self {
            uniform,
            skyview_view,
            transmittance_view,
            multiscatter_view,
            sampler,
            passes: Vec::new(),
            statics_done: false,
        };
        luts.build_passes(device, shader_source);
        luts
    }

    fn build_passes(&mut self, device: &wgpu::Device, shader_source: &str) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky luts"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // group 0 resources (inline per pipeline spec — wgpu::BindGroupEntry may not be Clone)
        // transmittance: uniform only
        let in_layout_tr = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cs_transmittance in"),
            entries: &[uniform_entry(0)],
        });
        let out_layout_tr = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cs_transmittance out"),
            entries: &[storage_entry(0, wgpu::TextureViewDimension::D2)],
        });
        let in_group_tr = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cs_transmittance in"),
            layout: &in_layout_tr,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            }],
        });
        let out_group_tr = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cs_transmittance out"),
            layout: &out_layout_tr,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&self.transmittance_view),
            }],
        });
        let layout_tr = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cs_transmittance"),
            bind_group_layouts: &[Some(&in_layout_tr), Some(&out_layout_tr)],
            immediate_size: 0,
        });
        let pipeline_tr = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cs_transmittance"),
            layout: Some(&layout_tr),
            module: &shader,
            entry_point: Some("cs_transmittance"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // multiscatter: uniform(0), transmittance sampled(1), sampler(3)
        let in_layout_ms = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cs_multiscatter in"),
            entries: &[uniform_entry(0), sampled_entry(1), sampler_entry(3)],
        });
        let out_layout_ms = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cs_multiscatter out"),
            entries: &[storage_entry(1, wgpu::TextureViewDimension::D2)],
        });
        let in_group_ms = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cs_multiscatter in"),
            layout: &in_layout_ms,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.transmittance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let out_group_ms = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cs_multiscatter out"),
            layout: &out_layout_ms,
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&self.multiscatter_view),
            }],
        });
        let layout_ms = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cs_multiscatter"),
            bind_group_layouts: &[Some(&in_layout_ms), Some(&out_layout_ms)],
            immediate_size: 0,
        });
        let pipeline_ms = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cs_multiscatter"),
            layout: Some(&layout_ms),
            module: &shader,
            entry_point: Some("cs_multiscatter"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // skyview: uniform(0), transmittance sampled(1), multiscatter sampled(2), sampler(3)
        let in_layout_sv = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cs_skyview in"),
            entries: &[uniform_entry(0), sampled_entry(1), sampled_entry(2), sampler_entry(3)],
        });
        let out_layout_sv = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cs_skyview out"),
            entries: &[storage_entry(2, wgpu::TextureViewDimension::D2)],
        });
        let in_group_sv = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cs_skyview in"),
            layout: &in_layout_sv,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.transmittance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.multiscatter_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let out_group_sv = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cs_skyview out"),
            layout: &out_layout_sv,
            entries: &[wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&self.skyview_view),
            }],
        });
        let layout_sv = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cs_skyview"),
            bind_group_layouts: &[Some(&in_layout_sv), Some(&out_layout_sv)],
            immediate_size: 0,
        });
        let pipeline_sv = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cs_skyview"),
            layout: Some(&layout_sv),
            module: &shader,
            entry_point: Some("cs_skyview"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        self.passes = vec![
            LutPass {
                pipeline: pipeline_tr,
                in_group: in_group_tr,
                out_group: out_group_tr,
                workgroups: (256 / 8, 64 / 8, 1),
            },
            LutPass {
                pipeline: pipeline_ms,
                in_group: in_group_ms,
                out_group: out_group_ms,
                workgroups: (32 / 8, 32 / 8, 1),
            },
            LutPass {
                pipeline: pipeline_sv,
                in_group: in_group_sv,
                out_group: out_group_sv,
                workgroups: (192 / 8, 108_u32.div_ceil(8), 1),
            },
        ];
    }

    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.build_passes(device, shader_source);
        self.statics_done = false; // recompute static LUTs with the new code
    }

    pub fn prepare(&self, queue: &wgpu::Queue, u: &AtmUniform) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(u));
    }

    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sky luts"),
            timestamp_writes,
        });
        let from = if self.statics_done { 2 } else { 0 };
        for pass in &self.passes[from..] {
            cpass.set_pipeline(&pass.pipeline);
            cpass.set_bind_group(0, &pass.in_group, &[]);
            cpass.set_bind_group(1, &pass.out_group, &[]);
            cpass.dispatch_workgroups(pass.workgroups.0, pass.workgroups.1, pass.workgroups.2);
        }
        self.statics_done = true;
    }
}

/// Earth-like atmosphere (Hillaire 2020 parameterization).
/// Distances in km, coefficients per km. The WGSL in sky_luts.wgsl mirrors
/// these constants — change BOTH or the sky stops matching the sun color.
pub struct Atmosphere {
    pub ground_radius: f32,
    pub top_radius: f32,
    pub rayleigh_scatter: Vec3,
    pub rayleigh_h: f32,
    pub mie_scatter: f32,
    pub mie_absorb: f32,
    pub mie_h: f32,
    pub ozone_absorb: Vec3,
    pub ozone_center: f32,
    pub ozone_half_width: f32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            ground_radius: 6360.0,
            top_radius: 6460.0,
            rayleigh_scatter: Vec3::new(5.802e-3, 13.558e-3, 33.1e-3),
            rayleigh_h: 8.0,
            mie_scatter: 3.996e-3,
            mie_absorb: 4.4e-3,
            mie_h: 1.2,
            ozone_absorb: Vec3::new(0.650e-3, 1.881e-3, 0.085e-3),
            ozone_center: 25.0,
            ozone_half_width: 15.0,
        }
    }
}

/// Top-of-atmosphere sun radiance. Tuned so the pre-exposure scene sits near
/// 1.0; auto-exposure (Task 11) makes the absolute scale uncritical.
pub const SUN_RADIANCE: Vec3 = Vec3::new(8.0, 7.6, 7.0);
/// Flat bluish moonlight (spec §7: "weak bluish directional moon light").
pub const MOON_RADIANCE: Vec3 = Vec3::new(0.012, 0.018, 0.032);

/// Nearest positive hit of the ray with a sphere of `radius` centered at the
/// planet origin; None when missed (or entirely behind the ray).
fn ray_sphere(origin: Vec3, dir: Vec3, radius: f32) -> Option<f32> {
    let b = origin.dot(dir);
    let c = origin.length_squared() - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    if -b - sq > 0.0 {
        Some(-b - sq)
    } else if -b + sq > 0.0 {
        Some(-b + sq)
    } else {
        None
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Atmosphere {
    fn extinction(&self, h: f32) -> Vec3 {
        let h = h.max(0.0);
        let rayl = (-h / self.rayleigh_h).exp();
        let mie = (-h / self.mie_h).exp();
        let ozone = (1.0 - (h - self.ozone_center).abs() / self.ozone_half_width).max(0.0);
        self.rayleigh_scatter * rayl
            + Vec3::splat((self.mie_scatter + self.mie_absorb) * mie)
            + self.ozone_absorb * ozone
    }

    /// Transmittance from `altitude` km above ground toward the sun at
    /// `cos_zenith`; zero when the ray hits the planet.
    pub fn transmittance(&self, altitude: f32, cos_zenith: f32) -> Vec3 {
        let pos = Vec3::new(0.0, self.ground_radius + altitude.max(1e-4), 0.0);
        let dir = Vec3::new((1.0 - cos_zenith * cos_zenith).max(0.0).sqrt(), cos_zenith, 0.0);
        if ray_sphere(pos, dir, self.ground_radius).is_some() {
            return Vec3::ZERO;
        }
        let Some(t_top) = ray_sphere(pos, dir, self.top_radius) else {
            return Vec3::ONE; // outside the atmosphere looking out
        };
        const STEPS: usize = 40;
        let dt = t_top / STEPS as f32;
        let mut depth = Vec3::ZERO;
        for i in 0..STEPS {
            let p = pos + dir * ((i as f32 + 0.5) * dt);
            depth += self.extinction(p.length() - self.ground_radius) * dt;
        }
        (-depth).exp()
    }
}

/// The frame's single analytic light: the transmittance-colored sun while it
/// is up, a weak bluish moon (direction -sun) otherwise. Both fade to ~zero
/// around the horizon, so the direction swap never pops visually.
/// Returns (direction toward the light, radiance, is_sun).
pub fn dominant_light(atm: &Atmosphere, sun_dir: Vec3, altitude_km: f32) -> (Vec3, Vec3, bool) {
    let sun_color = SUN_RADIANCE * atm.transmittance(altitude_km, sun_dir.y);
    let moon_dir = -sun_dir;
    let moon_color = MOON_RADIANCE * smoothstep(-0.04, 0.10, moon_dir.y);
    let lum = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    if lum(sun_color) >= lum(moon_color) {
        (sun_dir, sun_color, true)
    } else {
        (moon_dir, moon_color, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Vec3) -> f32 {
        c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
    }

    /// DayCycle-style sun direction at day fraction t (0 = sunrise).
    fn sun_at(t: f32) -> Vec3 {
        let a = t * std::f32::consts::TAU;
        Vec3::new(a.cos(), a.sin(), 0.12).normalize()
    }

    #[test]
    fn zenith_transmittance_is_high_and_reddens_in_order() {
        let t = Atmosphere::default().transmittance(0.2, 1.0);
        // Rayleigh scatters blue hardest: r > g > b survive in that order.
        assert!(t.x > t.y && t.y > t.z, "{t}");
        assert!(t.x > 0.9, "red barely touched at zenith: {t}");
        assert!((0.6..0.9).contains(&t.z), "blue optical depth ~0.26: {t}");
    }

    #[test]
    fn horizon_sun_is_strongly_red() {
        let atm = Atmosphere::default();
        let h = atm.transmittance(0.2, 0.0);
        assert!(h.x > h.z * 20.0, "sunset must be red-dominant: {h}");
        assert!(h.z < 0.05, "blue should be nearly gone at the horizon: {h}");
    }

    #[test]
    fn below_horizon_from_ground_level_is_black() {
        assert_eq!(Atmosphere::default().transmittance(0.0001, -0.1), Vec3::ZERO);
    }

    #[test]
    fn transmittance_is_monotonic_in_elevation() {
        let atm = Atmosphere::default();
        let mut prev = Vec3::ZERO;
        for i in 0..=20 {
            let t = atm.transmittance(0.1, i as f32 / 20.0);
            assert!(t.x >= prev.x - 1e-4 && t.z >= prev.z - 1e-4, "mu step {i}: {t} < {prev}");
            prev = t;
        }
    }

    #[test]
    fn noon_light_is_the_sun_midnight_light_is_a_blue_moon() {
        let atm = Atmosphere::default();
        let (dir, color, is_sun) = dominant_light(&atm, sun_at(0.25), 0.1);
        assert!(is_sun && dir.y > 0.9, "noon: sun overhead");
        assert!(color.x >= color.z && lum(color) > 1.0, "noon sun is warm and strong: {color}");

        let (dir, color, is_sun) = dominant_light(&atm, sun_at(0.75), 0.1);
        assert!(!is_sun && dir.y > 0.9, "midnight: moon overhead (moon = -sun)");
        assert!(color.z > color.x, "moonlight is bluish: {color}");
        assert!(lum(color) < 0.05, "moonlight is weak: {color}");
    }

    #[test]
    fn light_never_pops_through_sunset() {
        let atm = Atmosphere::default();
        let mut prev = dominant_light(&atm, sun_at(0.47), 0.1).1;
        for i in 1..=60 {
            let t = 0.47 + 0.06 * i as f32 / 60.0; // sweep through sunset
            let c = dominant_light(&atm, sun_at(t), 0.1).1;
            assert!(lum(c - prev).abs() < 0.25, "luminance jump at t={t}: {prev} -> {c}");
            prev = c;
        }
    }

    #[test]
    fn atm_uniform_layout_matches_wgsl() {
        // mat4 (64) + camera vec4 + sun vec4 + radiance vec4.
        assert_eq!(std::mem::size_of::<AtmUniform>(), 112);
        assert_eq!(std::mem::offset_of!(AtmUniform, camera), 64);
        assert_eq!(std::mem::offset_of!(AtmUniform, sun), 80);
        assert_eq!(std::mem::offset_of!(AtmUniform, sun_radiance), 96);
    }
}
