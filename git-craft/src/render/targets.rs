/// Offscreen render targets, recreated on resize. M5a: the HDR color target.
/// The bloom mip chain joins in the bloom task; GTAO/normals arrive in M5b.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// G-buffer: rgb = world normal (*0.5+0.5), a = ambient brightness fraction.
/// Rgba8Unorm (not the spec's RGB10A2): 8-bit alpha holds a continuous ambient
/// fraction that 2-bit RGB10A2 alpha cannot; render-attachment-capable.
pub const GBUF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Single-channel AO at half resolution. R8Unorm is render-attachment-capable
/// (unlike r8unorm storage), so GTAO runs as a fullscreen render pass.
pub const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Half resolution used by GTAO (and later volumetrics), clamped to >= 1.
pub fn half_size(width: u32, height: u32) -> (u32, u32) {
    ((width / 2).max(1), (height / 2).max(1))
}

/// Mip count for the half-res bloom chain: down to ~32 px, capped at 6.
pub fn bloom_mip_count(w: u32, h: u32) -> u32 {
    let (mut w, mut h, mut n) = (w, h, 1);
    while n < 6 && w >= 32 && h >= 32 {
        w /= 2;
        h /= 2;
        n += 1;
    }
    n
}

pub struct RenderTargets {
    pub hdr_view: wgpu::TextureView,
    /// G-buffer (normal + ambient weight), written by the main pass alongside HDR.
    pub gbuf_view: wgpu::TextureView,
    /// Raw half-res GTAO output (before the bilateral blur).
    pub ao_raw_view: wgpu::TextureView,
    /// Blurred half-res GTAO output (read by the composite pass).
    pub ao_blur_view: wgpu::TextureView,
    /// AO-composited HDR (main color × ambient-occlusion factor). TAA reads this.
    pub composited_view: wgpu::TextureView,
    /// Raw composited texture (COPY_SRC for the pre-water snapshot).
    pub composited_texture: wgpu::Texture,
    /// Snapshot of `composited_view` taken before the water pass, so water can
    /// sample the opaque scene behind it for refraction + SSR. COPY_DST.
    pub scene_color_view: wgpu::TextureView,
    /// Raw scene-color texture (COPY_DST target of the snapshot).
    pub scene_color_texture: wgpu::Texture,
    pub bloom_views: Vec<wgpu::TextureView>, // one per mip
    pub bloom_sizes: Vec<(u32, u32)>,
    pub width: u32,
    pub height: u32,
    /// TAA resolved output: read by bloom/exposure/post after the TAA pass.
    /// COPY_SRC so the TAA pass can copy resolved → history each frame.
    pub resolved_view: wgpu::TextureView,
    /// Raw resolved texture (needed by copy_texture_to_texture in TaaPass).
    pub resolved_texture: wgpu::Texture,
    /// TAA history ping-pong pair: [read_idx] is sampled this frame,
    /// [1-read_idx] receives the copy of resolved for next frame. COPY_DST.
    pub history_views: [wgpu::TextureView; 2],
    /// Raw history textures (needed by copy_texture_to_texture in TaaPass).
    pub history_textures: [wgpu::Texture; 2],
}

impl RenderTargets {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let hdr = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let gbuf = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gbuffer normal+ambient"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GBUF_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let (hw, hh) = half_size(width, height);
        let make_ao = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: hw,
                        height: hh,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: AO_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let ao_raw_view = make_ao("gtao raw");
        let ao_blur_view = make_ao("gtao blur");

        let half = ((width / 2).max(1), (height / 2).max(1));
        let mips = bloom_mip_count(half.0, half.1);
        let bloom = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bloom chain"),
            size: wgpu::Extent3d {
                width: half.0,
                height: half.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_views = (0..mips)
            .map(|i| {
                bloom.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("bloom mip"),
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let bloom_sizes = (0..mips)
            .map(|i| ((half.0 >> i).max(1), (half.1 >> i).max(1)))
            .collect();

        let resolved = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("taa resolved"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let history0 = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("taa history 0"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let history1 = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("taa history 1"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let composited = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ao composited hdr"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            // COPY_SRC: snapshotted into scene_color before the water pass.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let scene_color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scene color (water refraction source)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let history0_view = history0.create_view(&wgpu::TextureViewDescriptor::default());
        let history1_view = history1.create_view(&wgpu::TextureViewDescriptor::default());
        let resolved_view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            hdr_view: hdr.create_view(&wgpu::TextureViewDescriptor::default()),
            gbuf_view: gbuf.create_view(&wgpu::TextureViewDescriptor::default()),
            ao_raw_view,
            ao_blur_view,
            composited_view: composited.create_view(&wgpu::TextureViewDescriptor::default()),
            composited_texture: composited,
            scene_color_view: scene_color.create_view(&wgpu::TextureViewDescriptor::default()),
            scene_color_texture: scene_color,
            bloom_views,
            bloom_sizes,
            width,
            height,
            resolved_view,
            resolved_texture: resolved,
            history_views: [history0_view, history1_view],
            history_textures: [history0, history1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_size_clamps_to_one() {
        assert_eq!(half_size(1512, 982), (756, 491));
        assert_eq!(half_size(1, 1), (1, 1));
        assert_eq!(half_size(0, 0), (1, 1), "never zero");
    }

    #[test]
    fn bloom_mip_count_scales_with_resolution() {
        // Large inputs still cap at 6 (the while n<6 guard).
        assert_eq!(
            bloom_mip_count(1512, 982),
            6,
            "very large half-res still hits the cap of 6 mips"
        );
        assert_eq!(
            bloom_mip_count(20, 20),
            1,
            "20×20: below 32px threshold, stays at 1"
        );
        assert_eq!(bloom_mip_count(8, 8), 1, "never zero mips");
    }

    #[test]
    fn bloom_mip_count_five_mips_at_new_threshold() {
        // After raising the stop threshold to 32 px, 720p half-res (640×360)
        // yields 5 mips instead of 6 — saving 2 render passes.
        // Trace: 640×360→320×180→160×90→80×45→40×22 (stops: h 22 < 32), n=5.
        assert_eq!(
            bloom_mip_count(640, 360),
            5,
            "720p half-res: expect 5 mips with 32px threshold"
        );
        // 480p half-res: 320×180→160×90→80×45→40×22 (22<32), stops at n=4.
        assert_eq!(
            bloom_mip_count(320, 180),
            4,
            "480p half-res: expect 4 mips with 32px threshold"
        );
        // Very small windows: threshold ensures we still get at least 1 mip.
        assert_eq!(
            bloom_mip_count(16, 16),
            1,
            "16×16 half-res: threshold 32 means no iterations, stays at 1"
        );
        assert_eq!(
            bloom_mip_count(32, 32),
            2,
            "32×32 half-res: exactly at threshold, one iteration fires"
        );
    }
}
