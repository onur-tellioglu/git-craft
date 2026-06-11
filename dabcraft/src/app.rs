use std::collections::{HashMap, VecDeque};
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

/// Sections are drawn within this column radius: 12 × 32 = 384 blocks (spec §1).
const RENDER_RADIUS: i32 = 12;
/// Columns are generated one ring wider: meshing needs a full 3×3 neighborhood.
const LOAD_RADIUS: i32 = RENDER_RADIUS + 1;
/// Hysteresis: unload only beyond LOAD_RADIUS + 2 so walking along a column
/// border doesn't thrash gen/unload.
const UNLOAD_RADIUS: i32 = LOAD_RADIUS + 2;
/// In-flight job caps: keep the rayon queue short so newly-near work isn't
/// stuck behind a distant backlog (priority = submission order, spec §3).
const MAX_GEN_IN_FLIGHT: usize = 12;
const MAX_MESH_IN_FLIGHT: usize = 24;
/// GPU upload budget per frame (sections), to avoid frame spikes (spec §3).
const MAX_UPLOADS_PER_FRAME: usize = 24;
const SEED: i32 = 1337;

#[derive(Default)]
struct FrameStats {
    columns_ready: usize,
    visible_sections: u32,
    resident_sections: u32,
    drawn_quads: u32,
}

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
    occluded: bool,
    world: crate::world::chunks::ChunkMap,
    worldgen: crate::world::r#gen::WorldGen,
    jobs: crate::world::jobs::Jobs,
    upload_queue: VecDeque<(crate::world::chunks::SectionPos, Vec<crate::mesh::quad::PackedQuad>)>,
    /// Latest mesh-job version per section. Two jobs for one section can
    /// finish out of order; only a result matching the current version may
    /// be uploaded, or a stale snapshot would overwrite the fresh mesh.
    mesh_versions: HashMap<crate::world::chunks::SectionPos, u64>,
    stats: FrameStats,
}

impl App {
    pub fn new(instance: wgpu::Instance) -> Self {
        Self {
            instance: Some(instance),
            window: None,
            gpu: None,
            depth_view: None,
            input: InputState::default(),
            camera: Camera::new(glam::Vec3::new(16.0, 140.0, 16.0)),
            last_frame: std::time::Instant::now(),
            terrain: None,
            shader_watcher: None,
            egui: None,
            timer: None,
            hud_visible: true,
            fps_smoothed: 0.0,
            occluded: false,
            world: crate::world::chunks::ChunkMap::default(),
            worldgen: crate::world::r#gen::WorldGen::new(SEED),
            jobs: crate::world::jobs::Jobs::new(),
            upload_queue: VecDeque::new(),
            mesh_versions: HashMap::new(),
            stats: FrameStats::default(),
        }
    }

    /// One streaming step (spec §3, §4): drain finished jobs, unload far
    /// columns, request generation/meshing nearest-first under in-flight
    /// caps, upload finished meshes under a per-frame budget.
    /// ORDERING CONTRACT: drain BEFORE unload — a generation result dropped
    /// for distance leaves a Generating zombie slot that the unload pass in
    /// the same frame removes (drop condition radius == unload radius).
    fn update_world(&mut self) {
        use crate::world::chunks::{columns_in_radius, ColumnPos, SectionPos};
        use crate::world::jobs::JobResult;
        let Some(gpu) = self.gpu.as_ref() else { return };

        let center = ColumnPos {
            x: (self.camera.position.x as i32).div_euclid(32),
            z: (self.camera.position.z as i32).div_euclid(32),
        };

        // 1. Drain finished jobs.
        for result in self.jobs.drain() {
            match result {
                JobResult::Generated { pos, data, writes } => {
                    let d2 = (pos.x - center.x).pow(2) + (pos.z - center.z).pow(2);
                    if d2 > UNLOAD_RADIUS * UNLOAD_RADIUS {
                        // Player moved on; drop the data but keep its writes
                        // (the unload pass below removes the zombie slot).
                        self.world.queue_writes(writes);
                        continue;
                    }
                    self.world.insert_generated(pos, data, writes);
                }
                JobResult::Meshed { pos, version, quads } => {
                    let current = self.mesh_versions.get(&pos).copied().unwrap_or(0);
                    if version == current && self.world.ready(pos.column()).is_some() {
                        self.upload_queue.push_back((pos, quads));
                    }
                    // version < current: a newer job is in flight (or already
                    // landed) for this section — stale snapshot, drop it.
                }
            }
        }

        // 2. Unload far columns and free their GPU meshes + version entries.
        if let Some(terrain) = self.terrain.as_mut() {
            for pos in self.world.unload_outside(center, UNLOAD_RADIUS) {
                for y in 0..8 {
                    let section = SectionPos { x: pos.x, y, z: pos.z };
                    terrain.remove_section(section);
                    self.mesh_versions.remove(&section);
                }
            }
        }

        // 3. Request generation, nearest first.
        if self.jobs.gen_in_flight < MAX_GEN_IN_FLIGHT {
            for col in columns_in_radius(center, LOAD_RADIUS) {
                if self.jobs.gen_in_flight >= MAX_GEN_IN_FLIGHT {
                    break;
                }
                if !self.world.contains(col) {
                    self.world.mark_generating(col);
                    self.jobs.spawn_gen(self.worldgen.clone(), col);
                }
            }
        }

        // 4. Request meshing for dirty sections whose 3×3 columns are ready.
        if self.jobs.mesh_in_flight < MAX_MESH_IN_FLIGHT {
            'cols: for col in columns_in_radius(center, RENDER_RADIUS) {
                if self.world.ready(col).is_none() || !self.world.neighbors_ready(col) {
                    continue;
                }
                let dirty: Vec<usize> = self
                    .world
                    .ready(col)
                    .map(|c| (0..8).filter(|&y| c.dirty[y]).collect())
                    .unwrap_or_default();
                for sy in dirty {
                    if self.jobs.mesh_in_flight >= MAX_MESH_IN_FLIGHT {
                        break 'cols;
                    }
                    let pos = SectionPos { x: col.x, y: sy as i32, z: col.z };
                    let hood = self.build_neighborhood(pos);
                    if let Some(c) = self.world.ready_mut(col) {
                        c.dirty[sy] = false;
                    }
                    let version = self.mesh_versions.entry(pos).or_insert(0);
                    *version += 1;
                    self.jobs.spawn_mesh(pos, *version, hood);
                }
            }
        }

        // 5. Budgeted GPU uploads.
        if let Some(terrain) = self.terrain.as_mut() {
            for _ in 0..MAX_UPLOADS_PER_FRAME {
                let Some((pos, quads)) = self.upload_queue.pop_front() else { break };
                if self.world.ready(pos.column()).is_none() {
                    continue; // unloaded while queued
                }
                terrain.upload_section(&gpu.queue, pos, &quads);
            }
        }

        self.stats.columns_ready = self.world.ready_count();
    }

    /// Capture the 3×3×3 Arc<Section> neighborhood around a section.
    fn build_neighborhood(
        &self,
        pos: crate::world::chunks::SectionPos,
    ) -> crate::mesh::neighborhood::MeshNeighborhood {
        use crate::mesh::neighborhood::MeshNeighborhood;
        use crate::world::chunks::ColumnPos;
        let mut hood = MeshNeighborhood::empty();
        for dy in -1..=1 {
            let sy = pos.y + dy;
            if !(0..8).contains(&sy) {
                continue; // above/below the world: stays None → air
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let col = ColumnPos { x: pos.x + dx, z: pos.z + dz };
                    if let Some(c) = self.world.ready(col) {
                        hood.sections[MeshNeighborhood::index(dx, dy, dz)] =
                            Some(c.sections[sy as usize].clone());
                    }
                }
            }
        }
        hood
    }

    fn render(&mut self) {
        // Guards first: input must not be consumed while the GPU is still initializing.
        if self.gpu.is_none() || self.depth_view.is_none() {
            return;
        }

        // Hot-reload: poll shader file; swap pipeline only if naga validation passes.
        if let (Some(watcher), Some(terrain), Some(gpu)) =
            (self.shader_watcher.as_mut(), self.terrain.as_mut(), self.gpu.as_ref())
            && let Some(source) = watcher.poll() {
                terrain.swap_shader(&gpu.device, &source);
            }

        let now = std::time::Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        // Smooth FPS estimate.
        self.fps_smoothed = self.fps_smoothed * 0.95 + (1.0 / dt.max(1e-6)) * 0.05;

        let (dx, dy) = self.input.take_mouse_delta();
        self.camera.apply_mouse_delta(dx, dy);
        self.camera.fly(&self.input, dt);

        // World streaming: gen/mesh/upload jobs.
        self.update_world();

        // Disjoint field borrows: terrain mutably for prepare, gpu mutably.
        let Some(depth_view_ref) = self.depth_view.as_ref() else { return };
        let Some(gpu) = self.gpu.as_mut() else { return };

        if let Some(terrain) = self.terrain.as_mut() {
            let aspect = gpu.config.width as f32 / gpu.config.height as f32;
            let view_proj = self.camera.view_proj(aspect);
            terrain.write_camera(&gpu.queue, view_proj);
            let frustum = crate::render::frustum::Frustum::from_view_proj(view_proj);
            let stats = terrain.prepare(&gpu.queue, &frustum);
            self.stats.visible_sections = stats.visible_sections;
            self.stats.resident_sections = stats.resident_sections;
            self.stats.drawn_quads = stats.drawn_quads;
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
            // Reborrow immutably for draw (prepare has already finished above).
            if let Some(terrain) = self.terrain.as_ref() {
                terrain.draw(&mut rpass);
            }
        }

        // Resolve GPU timestamps into the readback buffer.
        if let Some(timer) = &self.timer {
            timer.resolve(&mut encoder);
        }

        // Capture all values before the egui closure to avoid borrowing self.
        let fps = self.fps_smoothed;
        let gpu_ms = self.timer.as_ref().map(|t| t.last_ms).unwrap_or(0.0);
        let cam = self.camera.position;
        let cols = self.stats.columns_ready;
        let visible = self.stats.visible_sections;
        let resident = self.stats.resident_sections;
        let quads = self.stats.drawn_quads;
        let gen_q = self.jobs.gen_in_flight;
        let mesh_q = self.jobs.mesh_in_flight;
        let uploads = self.upload_queue.len();
        let (arena_used, arena_cap) =
            self.terrain.as_ref().map(|t| t.arena_usage()).unwrap_or((0, 1));

        // Draw egui HUD overlay.
        let egui_cmds = if self.hud_visible {
            if let Some(egui) = &mut self.egui {
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
                                ui.label(format!("FPS:      {fps:.1}"));
                                ui.label(format!("GPU ms:   {gpu_ms:.2}"));
                                ui.label(format!("Pos:      {:.0} {:.0} {:.0}", cam.x, cam.y, cam.z));
                                ui.label(format!("Columns:  {cols}"));
                                ui.label(format!("Sections: {visible}/{resident} drawn/resident"));
                                ui.label(format!("Quads:    {quads}"));
                                ui.label(format!("Jobs:     gen {gen_q}  mesh {mesh_q}  upload {uploads}"));
                                ui.label(format!(
                                    "Arena:    {:.1}/{:.0} MiB",
                                    arena_used as f32 * 8.0 / (1 << 20) as f32,
                                    arena_cap as f32 * 8.0 / (1 << 20) as f32
                                ));
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

        // Submit order matters: egui's user_cmds carry staging uploads that
        // must execute before main_cmd, whose egui render pass samples them.
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
        let terrain = TerrainRenderer::new(&gpu.device, gpu.config.format, &shader_source);
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
        if let (Some(egui), Some(window)) = (&mut self.egui, &self.window)
            && egui.on_window_event(window, &event) {
                return;
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
            WindowEvent::Occluded(occluded) => {
                // macOS doesn't block get_current_texture for hidden windows;
                // stop requesting redraws to avoid spinning the CPU (spec §2).
                self.occluded = occluded;
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
        if self.occluded {
            return;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
