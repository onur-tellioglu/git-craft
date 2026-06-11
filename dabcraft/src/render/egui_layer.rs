use std::sync::Arc;

use winit::window::Window;

pub struct EguiLayer {
    pub ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

impl EguiLayer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat, window: &Window) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(
            device,
            surface_format,
            egui_wgpu::RendererOptions::default(),
        );
        Self { ctx, state, renderer }
    }

    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// Discard accumulated input. Must be called every frame the HUD is not
    /// drawn — on_window_event buffers events until take_egui_input drains
    /// them, and a long-hidden HUD would otherwise replay the whole backlog.
    pub fn drain_input(&mut self, window: &Window) {
        let _ = self.state.take_egui_input(window);
    }

    // The argument list is wgpu frame plumbing (device/queue/encoder/target);
    // bundling it into a context struct would add a type for one call site.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        window: &Arc<Window>,
        view: &wgpu::TextureView,
        config: &wgpu::SurfaceConfiguration,
        mut ui: impl FnMut(&egui::Context),
    ) -> Vec<wgpu::CommandBuffer> {
        let raw_input = self.state.take_egui_input(window);
        self.ctx.begin_pass(raw_input);
        ui(&self.ctx);
        let output = self.ctx.end_pass();
        self.state.handle_platform_output(window, output.platform_output);

        let paint_jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [config.width, config.height],
            pixels_per_point: window.scale_factor() as f32,
        };
        for (id, delta) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }
        let user_cmds = self.renderer.update_buffers(device, queue, encoder, &paint_jobs, &screen);
        {
            let mut rpass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                // Required: egui_wgpu::Renderer::render takes &mut RenderPass<'static>
                // (invariant), so the pass lifetime must be erased. Do not touch
                // `encoder` while `rpass` is alive.
                .forget_lifetime();
            self.renderer.render(&mut rpass, &paint_jobs, &screen);
        }
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        user_cmds
    }
}
