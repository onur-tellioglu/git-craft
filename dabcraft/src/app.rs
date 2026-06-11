use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::render::depth;
use crate::render::gpu::Gpu;

pub struct App {
    // Taken by value when the GPU context is created on first resume.
    instance: Option<wgpu::Instance>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    depth_view: Option<wgpu::TextureView>,
}

impl App {
    pub fn new(instance: wgpu::Instance) -> Self {
        Self { instance: Some(instance), window: None, gpu: None, depth_view: None }
    }

    fn render(&mut self) {
        let depth_view = match self.depth_view.as_ref() {
            Some(v) => v as *const wgpu::TextureView,
            None => return,
        };
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(frame) = gpu.acquire() else { return };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            // SAFETY: depth_view is behind self.depth_view (Option<TextureView>) which lives
            // as long as self. We hold an exclusive borrow on self via &mut self, so no
            // aliasing is possible. The raw pointer is used only to satisfy the borrow checker
            // when self.gpu is also borrowed mutably.
            let depth_view_ref = unsafe { &*depth_view };
            let _rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.25, g: 0.55, b: 0.95, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view_ref,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard, // TBDR: not sampled later on M1
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        gpu.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return; // macOS can resume more than once; init exactly once
        }
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("dabcraft"))
                .unwrap(),
        );
        let instance = self.instance.take().expect("resumed twice with GPU already built");
        let gpu = Gpu::new(&instance, window.clone());
        let size = window.inner_size();
        self.depth_view = Some(depth::create_depth_view(&gpu.device, size.width, size.height));
        self.gpu = Some(gpu);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    && event.state.is_pressed()
                    && !event.repeat
                {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                    self.depth_view = Some(depth::create_depth_view(&gpu.device, size.width, size.height));
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, _event: DeviceEvent) {}

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
