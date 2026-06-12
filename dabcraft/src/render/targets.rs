/// Offscreen render targets, recreated on resize. M5a: the HDR color target.
/// The bloom mip chain joins in the bloom task; GTAO/normals arrive in M5b.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct RenderTargets {
    pub hdr_view: wgpu::TextureView,
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
        Self { hdr_view: hdr.create_view(&wgpu::TextureViewDescriptor::default()) }
    }
}
