use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::game::camera::Camera;
use crate::game::input::InputState;
use crate::render::depth;
use crate::render::gpu::Gpu;
use crate::render::terrain::TerrainRenderer;

pub struct App {
    // Taken by value when the GPU context is created on first resume.
    instance: Option<wgpu::Instance>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    depth_view: Option<wgpu::TextureView>,
    input: InputState,
    camera: Camera,
    last_frame: std::time::Instant,
    terrain: Option<TerrainRenderer>,
    shader_watcher: Option<crate::render::hot_reload::ShaderWatcher>,
}

impl App {
    pub fn new(instance: wgpu::Instance) -> Self {
        Self {
            instance: Some(instance),
            window: None,
            gpu: None,
            depth_view: None,
            input: InputState::default(),
            camera: Camera::new(glam::Vec3::new(16.0, 40.0, 60.0)),
            last_frame: std::time::Instant::now(),
            terrain: None,
            shader_watcher: None,
        }
    }

    fn render(&mut self) {
        // Guards first: input must not be consumed while the GPU is still initializing.
        if self.gpu.is_none() || self.depth_view.is_none() {
            return;
        }

        // Hot-reload: poll shader file; swap pipeline only if naga validation passes.
        if let (Some(watcher), Some(terrain), Some(gpu)) =
            (self.shader_watcher.as_mut(), self.terrain.as_mut(), self.gpu.as_ref())
        {
            if let Some(source) = watcher.poll() {
                terrain.swap_shader(&gpu.device, &source);
            }
        }

        let now = std::time::Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let (dx, dy) = self.input.take_mouse_delta();
        self.camera.apply_mouse_delta(dx, dy);
        self.camera.fly(&self.input, dt);

        // Disjoint field borrows: terrain + depth_view immutably, gpu mutably.
        let terrain = self.terrain.as_ref();
        let Some(depth_view_ref) = self.depth_view.as_ref() else { return };
        let Some(gpu) = self.gpu.as_mut() else { return };

        if let Some(terrain) = terrain {
            let aspect = gpu.config.width as f32 / gpu.config.height as f32;
            terrain.write_camera(&gpu.queue, self.camera.view_proj(aspect));
        }

        let Some(frame) = gpu.acquire() else { return };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            if let Some(terrain) = terrain {
                terrain.draw(&mut rpass);
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

fn build_test_section() -> crate::world::section::Section {
    use crate::world::block::{DIRT, GRASS, STONE};
    let mut s = crate::world::section::Section::empty();
    for x in 0..32 {
        for z in 0..32 {
            for y in 0..3 {
                s.set(x, y, z, STONE);
            }
            s.set(x, 3, z, DIRT);
            s.set(x, 4, z, GRASS);
        }
    }
    // landmarks: a pillar and a floating cube to judge depth and faces
    for y in 5..12 {
        s.set(8, y, 8, STONE);
    }
    for x in 20..24 {
        for y in 8..12 {
            for z in 20..24 {
                s.set(x, y, z, DIRT);
            }
        }
    }
    s
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

        let shader_path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/terrain.wgsl");
        // Watcher first: its baseline mtime must predate the source read, so a
        // save landing in between is detected as a change instead of missed.
        self.shader_watcher = Some(crate::render::hot_reload::ShaderWatcher::new(shader_path));
        let shader_source = std::fs::read_to_string(shader_path).expect("terrain.wgsl missing");
        let mut terrain = TerrainRenderer::new(&gpu.device, gpu.config.format, &shader_source);
        terrain.upload_quads(&gpu.device, &crate::mesh::naive::mesh_naive(&build_test_section()));
        self.terrain = Some(terrain);

        self.gpu = Some(gpu);
        if window.set_cursor_grab(winit::window::CursorGrabMode::Locked).is_err() {
            // Locked is unsupported on some platforms (e.g. X11); Confined is the fallback.
            let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Confined);
        }
        window.set_cursor_visible(false);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if code == KeyCode::Escape && event.state.is_pressed() && !event.repeat {
                        event_loop.exit();
                    }
                    self.input.set_key(code, event.state.is_pressed());
                }
            }
            WindowEvent::Focused(_) => {
                // Drop held keys and stale mouse deltas on any focus transition.
                self.input.clear();
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

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.input.accumulate_mouse(delta.0, delta.1);
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
