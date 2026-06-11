use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::game::camera::Camera;
use crate::game::input::InputState;
use crate::render::depth;
use crate::render::egui_layer::EguiLayer;
use crate::render::gpu::Gpu;
use crate::render::terrain::TerrainRenderer;
use crate::render::timestamps::GpuTimer;

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
    egui: Option<EguiLayer>,
    timer: Option<GpuTimer>,
    hud_visible: bool,
    fps_smoothed: f32,
    quad_count: u32,
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
            egui: None,
            timer: None,
            hud_visible: true,
            fps_smoothed: 0.0,
            quad_count: 0,
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

        // Smooth FPS estimate.
        self.fps_smoothed = self.fps_smoothed * 0.95 + (1.0 / dt.max(1e-6)) * 0.05;

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

        // Capture timestamp_writes before the block to avoid borrow issues.
        let ts_writes = self.timer.as_ref().and_then(|t| t.pass_writes());

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
                timestamp_writes: ts_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(terrain) = terrain {
                terrain.draw(&mut rpass);
            }
        }

        // Resolve GPU timestamps into the readback buffer.
        if let Some(timer) = &self.timer {
            timer.resolve(&mut encoder);
        }

        // Draw egui HUD overlay.
        let egui_cmds = if self.hud_visible {
            if let Some(egui) = &mut self.egui {
                let fps = self.fps_smoothed;
                let gpu_ms = self.timer.as_ref().map(|t| t.last_ms).unwrap_or(0.0);
                let quads = self.quad_count;
                let window = self.window.as_ref().unwrap().clone();
                let config = &gpu.config;
                let cmds = egui.draw(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    &window,
                    &view,
                    config,
                    |ctx| {
                        egui::Window::new("Debug HUD")
                            .resizable(false)
                            .collapsible(false)
                            .show(ctx, |ui| {
                                ui.label(format!("FPS:    {:.1}", fps));
                                ui.label(format!("GPU ms: {:.2}", gpu_ms));
                                ui.label(format!("Quads:  {}", quads));
                            });
                    },
                );
                Some(cmds)
            } else {
                None
            }
        } else {
            // HUD hidden: still drain egui's buffered input so re-enabling it
            // doesn't replay a backlog of stale events in one frame.
            if let (Some(egui), Some(window)) = (&mut self.egui, &self.window) {
                egui.drain_input(window);
            }
            None
        };

        // Submit: egui user command buffers first, then the main encoder.
        let main_cmd = encoder.finish();
        let mut all_cmds: Vec<wgpu::CommandBuffer> = Vec::new();
        if let Some(mut cmds) = egui_cmds {
            all_cmds.append(&mut cmds);
        }
        all_cmds.push(main_cmd);
        gpu.queue.submit(all_cmds);

        // Poll for async timestamp readback.
        if let Some(timer) = &mut self.timer {
            timer.after_submit(&gpu.device, &gpu.queue);
        }

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
        let quads = crate::mesh::naive::mesh_naive(&build_test_section());
        self.quad_count = quads.len() as u32;
        terrain.upload_quads(&gpu.device, &quads);
        self.terrain = Some(terrain);

        // Initialize egui and GPU timer.
        self.egui = Some(EguiLayer::new(&gpu.device, gpu.config.format, &window));
        self.timer = Some(GpuTimer::new(&gpu.device));

        self.gpu = Some(gpu);
        if window.set_cursor_grab(winit::window::CursorGrabMode::Locked).is_err() {
            // Locked is unsupported on some platforms (e.g. X11); Confined is the fallback.
            let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Confined);
        }
        window.set_cursor_visible(false);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // System-critical events are handled before (and regardless of) the
        // egui filter: a focused widget must never swallow exit, HUD toggle,
        // resize, or the redraw that drives the frame loop.
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                self.render();
                return;
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                    self.depth_view =
                        Some(depth::create_depth_view(&gpu.device, size.width, size.height));
                }
                return;
            }
            WindowEvent::KeyboardInput { event: key, .. }
                if key.state.is_pressed() && !key.repeat =>
            {
                match key.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => {
                        event_loop.exit();
                        return;
                    }
                    PhysicalKey::Code(KeyCode::F3) => {
                        self.hud_visible = !self.hud_visible;
                        return;
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // Feed egui next; if it consumes the event, don't propagate to game input.
        if let (Some(egui), Some(window)) = (&mut self.egui, &self.window) {
            if egui.on_window_event(window, &event) {
                return;
            }
        }

        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.input.set_key(code, event.state.is_pressed());
                }
            }
            WindowEvent::Focused(_) => {
                // Drop held keys and stale mouse deltas on any focus transition.
                self.input.clear();
            }
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
