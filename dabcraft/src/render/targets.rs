/// Offscreen render targets, recreated on resize. M5a: the HDR color target.
/// The bloom mip chain joins in the bloom task; GTAO/normals arrive in M5b.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Mip count for the half-res bloom chain: down to ~8 px, capped at 6.
pub fn bloom_mip_count(w: u32, h: u32) -> u32 {
    let (mut w, mut h, mut n) = (w, h, 1);
    while n < 6 && w >= 16 && h >= 16 {
        w /= 2;
        h /= 2;
        n += 1;
    }
    n
}

pub struct RenderTargets {
    pub hdr_view: wgpu::TextureView,
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
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let half = ((width / 2).max(1), (height / 2).max(1));
        let mips = bloom_mip_count(half.0, half.1);
        let bloom = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bloom chain"),
            size: wgpu::Extent3d { width: half.0, height: half.1, depth_or_array_layers: 1 },
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
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
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
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
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
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let history0_view = history0.create_view(&wgpu::TextureViewDescriptor::default());
        let history1_view = history1.create_view(&wgpu::TextureViewDescriptor::default());
        let resolved_view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            hdr_view: hdr.create_view(&wgpu::TextureViewDescriptor::default()),
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
    fn bloom_mip_count_scales_with_resolution() {
        assert_eq!(bloom_mip_count(1512, 982), 6, "native half-res gets the full chain");
        assert_eq!(bloom_mip_count(20, 20), 2);
        assert_eq!(bloom_mip_count(8, 8), 1, "never zero mips");
    }
}
