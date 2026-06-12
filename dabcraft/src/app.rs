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

/// GPU pass timing slots (spec §8). Order is frame order; indices are stable
/// within a task but renumbered as the frame graph grows through M5.
const PASS_LABELS: &[&str] = &["shadow0", "shadow1", "shadow2", "main", "post"];
const PASS_SHADOW0: usize = 0;
const PASS_MAIN: usize = 3;
const PASS_POST: usize = 4;

fn shader_path(name: &str) -> String {
    format!("{}/assets/shaders/{name}", env!("CARGO_MANIFEST_DIR"))
}

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
/// Block interaction reach from the eye (spec §7).
const REACH: f32 = 6.0;
/// Held-button break/place repeat interval (creative).
const EDIT_REPEAT: f32 = 0.25;
/// Two Space presses within this window toggle walk/fly.
const DOUBLE_TAP_WINDOW: f32 = 0.35;

#[derive(Default)]
struct FrameStats {
    columns_ready: usize,
    visible_sections: u32,
    resident_sections: u32,
    drawn_quads: u32,
    cave_culled: u32,
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
    shadow: Option<crate::render::shadow::ShadowRenderer>,
    shaders: Option<crate::render::hot_reload::ShaderSet>,
    targets: Option<crate::render::targets::RenderTargets>,
    post: Option<crate::render::post::PostPass>,
    egui: Option<EguiLayer>,
    timer: Option<GpuTimer>,
    hud_visible: bool,
    fps_smoothed: f32,
    occluded: bool,
    player: crate::game::player::Player,
    hotbar: crate::game::hotbar::Hotbar,
    outline: Option<crate::render::outline::OutlineRenderer>,
    target: Option<crate::game::raycast::RayHit>,
    last_space_press: Option<std::time::Instant>,
    break_timer: f32,
    place_timer: f32,
    cursor_grabbed: bool,
    day: crate::game::daycycle::DayCycle,
    world: crate::world::chunks::ChunkMap,
    worldgen: crate::world::r#gen::WorldGen,
    jobs: crate::world::jobs::Jobs,
    upload_queue: VecDeque<(crate::world::chunks::SectionPos, Vec<crate::mesh::quad::PackedQuad>)>,
    /// Latest mesh-job version per section. Two jobs for one section can
    /// finish out of order; only a result matching the current version may
    /// be uploaded, or a stale snapshot would overwrite the fresh mesh.
    mesh_versions: HashMap<crate::world::chunks::SectionPos, u64>,
    /// Face-connectivity mask per meshed section (cave culling, spec §6).
    visibility_masks: HashMap<crate::world::chunks::SectionPos, u16>,
    cave_culling: bool,
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
            shadow: None,
            shaders: None,
            targets: None,
            post: None,
            egui: None,
            timer: None,
            hud_visible: true,
            fps_smoothed: 0.0,
            occluded: false,
            player: crate::game::player::Player::new(glam::Vec3::new(16.0, 140.0, 16.0)),
            hotbar: crate::game::hotbar::Hotbar::new(),
            outline: None,
            target: None,
            last_space_press: None,
            break_timer: 0.0,
            place_timer: 0.0,
            cursor_grabbed: false,
            day: crate::game::daycycle::DayCycle::new(),
            world: crate::world::chunks::ChunkMap::default(),
            worldgen: crate::world::r#gen::WorldGen::new(SEED),
            jobs: crate::world::jobs::Jobs::new(),
            upload_queue: VecDeque::new(),
            mesh_versions: HashMap::new(),
            visibility_masks: HashMap::new(),
            cave_culling: true,
            stats: FrameStats::default(),
        }
    }

    /// Grab (lock + hide) or release the cursor. Input state is cleared on
    /// both transitions so half-held keys/buttons don't leak across.
    fn set_cursor_grab(&mut self, grab: bool) {
        let Some(window) = &self.window else { return };
        if grab {
            if window.set_cursor_grab(winit::window::CursorGrabMode::Locked).is_err() {
                let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Confined);
            }
            window.set_cursor_visible(false);
        } else {
            let _ = window.set_cursor_grab(winit::window::CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
        self.cursor_grabbed = grab;
        self.input.clear();
        // A Space tap before the transition must not pair with one after it.
        self.last_space_press = None;
    }

    /// Raycast the targeted block and apply break/place edits. Edits call
    /// ChunkMap::set_block, whose dirty flags feed the existing M2 re-mesh
    /// path in update_world (versioned jobs drop any stale in-flight mesh).
    fn update_interaction(&mut self, dt: f32) {
        use crate::game::input::MouseButton;
        use crate::world::block::{AIR, WATER};

        self.break_timer = (self.break_timer - dt).max(0.0);
        self.place_timer = (self.place_timer - dt).max(0.0);

        // Target anything non-air: water is visually opaque until M5, so
        // targeting (and breaking) it matches what the player sees.
        self.target = {
            let world = &self.world;
            let hits = |c: glam::IVec3| world.block_at(c).is_some_and(|b| b != AIR);
            crate::game::raycast::raycast(self.camera.position, self.camera.forward(), REACH, &hits)
        };
        if !self.cursor_grabbed {
            return;
        }
        let Some(hit) = self.target else { return };

        // Left: instant break (creative), repeating while held.
        if self.input.mouse_pressed(MouseButton::Left)
            || (self.input.mouse_down(MouseButton::Left) && self.break_timer == 0.0)
        {
            if self.world.set_block(hit.block, AIR) {
                crate::world::light_engine::on_block_changed(&mut self.world, hit.block);
            }
            self.break_timer = EDIT_REPEAT;
        }

        // Right: place against the hit face. No face when the ray started
        // inside a block. Rejected if the cell is occupied (water counts as
        // replaceable) or intersects the player AABB (spec §7).
        if (self.input.mouse_pressed(MouseButton::Right)
            || (self.input.mouse_down(MouseButton::Right) && self.place_timer == 0.0))
            && hit.normal != glam::IVec3::ZERO
        {
            let cell = hit.block + hit.normal;
            let free = self.world.block_at(cell).is_some_and(|b| b == AIR || b == WATER);
            if free && !self.player.aabb().intersects_cell(cell) {
                if self.world.set_block(cell, self.hotbar.selected_block()) {
                    crate::world::light_engine::on_block_changed(&mut self.world, cell);
                }
                self.place_timer = EDIT_REPEAT;
            }
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
                JobResult::Generated { pos, data, light, writes } => {
                    let d2 = (pos.x - center.x).pow(2) + (pos.z - center.z).pow(2);
                    if d2 > UNLOAD_RADIUS * UNLOAD_RADIUS {
                        // Player moved on; drop the data but keep its writes
                        // (the unload pass below removes the zombie slot).
                        self.world.queue_writes(writes);
                        continue;
                    }
                    let touched = self.world.insert_generated(pos, data, *light, writes);
                    // Heal the light seams against already-loaded neighbors,
                    // then fix light under structure writes that landed after
                    // the gen job lit the column.
                    crate::world::light_engine::seed_column_borders(&mut self.world, pos);
                    for p in touched {
                        crate::world::light_engine::on_block_changed(&mut self.world, p);
                    }
                }
                JobResult::Meshed { pos, version, quads, visibility } => {
                    let current = self.mesh_versions.get(&pos).copied().unwrap_or(0);
                    if version == current && self.world.ready(pos.column()).is_some() {
                        self.visibility_masks.insert(pos, visibility);
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
                    self.visibility_masks.remove(&section);
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
                        hood.light[MeshNeighborhood::index(dx, dy, dz)] =
                            Some(c.light[sy as usize].clone());
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

        // Hot-reload: poll all shader files; swap pipelines only if naga validation passes.
        if let (Some(set), Some(gpu)) = (self.shaders.as_mut(), self.gpu.as_ref()) {
            for (name, source) in set.poll() {
                match name {
                    "terrain" => {
                        if let Some(t) = self.terrain.as_mut() {
                            t.swap_shader(&gpu.device, &source);
                        }
                    }
                    "post" => {
                        if let Some(p) = self.post.as_mut() {
                            p.swap_shader(&gpu.device, &source);
                        }
                    }
                    "shadow" => {
                        if let (Some(s), Some(t)) = (self.shadow.as_mut(), self.terrain.as_ref()) {
                            s.swap_shader(&gpu.device, t.quads_layout(), &source);
                        }
                    }
                    // outline has no swap_shader yet; restart to pick it up.
                    _ => {}
                }
            }
        }

        let now = std::time::Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        // Smooth FPS estimate.
        self.fps_smoothed = self.fps_smoothed * 0.95 + (1.0 / dt.max(1e-6)) * 0.05;

        // Advance day/night cycle unconditionally — time flows even while paused.
        self.day.advance(dt);

        let (dx, dy) = self.input.take_mouse_delta();

        // Released cursor = paused: the world keeps streaming, but the
        // player, camera, and all gameplay input are frozen until the
        // click-to-refocus re-grab.
        if self.cursor_grabbed {
            self.camera.apply_mouse_delta(dx, dy);

            // Mode toggles: F, or double-tapped Space (spec §7).
            if self.input.key_pressed(KeyCode::KeyF) {
                self.player.toggle_mode();
            }
            if self.input.key_pressed(KeyCode::KeyV) {
                self.cave_culling = !self.cave_culling;
            }
            if self.input.key_pressed(KeyCode::Space) {
                let now = std::time::Instant::now();
                if self
                    .last_space_press
                    .is_some_and(|t| now.duration_since(t).as_secs_f32() < DOUBLE_TAP_WINDOW)
                {
                    self.player.toggle_mode();
                    self.last_space_press = None;
                } else {
                    self.last_space_press = Some(now);
                }
            }

            // Hotbar: 1–9 select, wheel cycles, shift+wheel pages (spec §7).
            const DIGITS: [KeyCode; 9] = [
                KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3,
                KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6,
                KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9,
            ];
            for (i, key) in DIGITS.iter().enumerate() {
                if self.input.key_pressed(*key) {
                    self.hotbar.select(i);
                }
            }
            let scroll_steps = self.input.take_scroll_steps();
            if scroll_steps != 0 {
                if self.input.is_down(KeyCode::ShiftLeft) {
                    self.hotbar.page_scroll(scroll_steps);
                } else {
                    self.hotbar.scroll(scroll_steps);
                }
            }

            // Player movement against the loaded world. Unloaded columns are
            // solid: the player floats at the load edge instead of falling
            // through terrain that hasn't generated yet.
            {
                let world = &self.world;
                let is_solid = |c: glam::IVec3| match world.block_at(c) {
                    Some(b) => b != crate::world::block::AIR && b != crate::world::block::WATER,
                    None => true,
                };
                let is_water =
                    |c: glam::IVec3| world.block_at(c) == Some(crate::world::block::WATER);
                self.player.update(&self.input, self.camera.yaw, dt, &is_solid, &is_water);
            }
            self.camera.position = self.player.eye();

            self.update_interaction(dt);
        } else {
            // No targeted block while paused: hides the outline.
            self.target = None;
        }

        // World streaming: gen/mesh/upload jobs.
        self.update_world();

        // Disjoint field borrows: terrain mutably for prepare, gpu mutably.
        let Some(depth_view_ref) = self.depth_view.as_ref() else { return };
        let Some(gpu) = self.gpu.as_mut() else { return };

        let aspect = gpu.config.width as f32 / gpu.config.height as f32;
        let view_proj = self.camera.view_proj(aspect);

        // Compute cave-culling visible set BEFORE borrowing terrain mutably.
        // Capture only the fields we need so the closure stays disjoint from
        // the mutable terrain borrow below.
        let cam_pos = self.camera.position;
        let cave_culling = self.cave_culling;
        let masks = &self.visibility_masks;
        let frustum = crate::render::frustum::Frustum::from_view_proj(view_proj);
        let visible = if cave_culling {
            let cam_section = crate::world::chunks::SectionPos {
                x: (cam_pos.x as i32).div_euclid(32),
                y: (cam_pos.y as i32).div_euclid(32),
                z: (cam_pos.z as i32).div_euclid(32),
            };
            Some(crate::render::visibility::visible_set(cam_section, RENDER_RADIUS, |p| {
                masks.get(&p).copied()
            }))
        } else {
            None
        };

        if let Some(terrain) = self.terrain.as_mut() {
            terrain.write_frame(
                &gpu.queue,
                view_proj,
                self.day.sky_color(),
                self.day.day_factor(),
                self.day.sun_dir(),
            );
            let stats = terrain.prepare(&gpu.queue, &frustum, visible.as_ref());
            self.stats.visible_sections = stats.visible_sections;
            self.stats.resident_sections = stats.resident_sections;
            self.stats.drawn_quads = stats.drawn_quads;
            self.stats.cave_culled = stats.cave_culled;
        }
        if let Some(outline) = self.outline.as_mut() {
            outline.set_target(&gpu.queue, view_proj, self.target.map(|h| h.block));
        }

        let light_dir = self.day.sun_dir(); // Task 6 replaces with sun-or-moon
        if let (Some(shadow), Some(terrain)) = (self.shadow.as_mut(), self.terrain.as_ref()) {
            shadow.prepare(
                &gpu.queue,
                terrain,
                self.camera.position,
                self.camera.forward(),
                self.camera.fov_y,
                aspect,
                light_dir,
            );
        }

        let sky = self.day.sky_color();
        let Some(frame) = gpu.acquire() else {
            // Press edges were consumed by this frame's logic above; clear them
            // even when the swapchain frame is dropped, or a click would fire
            // a second edit on the next frame.
            self.input.end_frame();
            return;
        };
        let Some(targets) = self.targets.as_ref() else { return };
        let hdr_view = &targets.hdr_view;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

        if let (Some(shadow), Some(terrain)) = (self.shadow.as_mut(), self.terrain.as_ref()) {
            shadow.encode(&mut encoder, terrain, self.timer.as_ref(), PASS_SHADOW0);
        }

        // Capture timestamp_writes before the block to avoid borrow issues.
        let ts_writes = self.timer.as_ref().and_then(|t| t.render_writes(PASS_MAIN));

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: hdr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: sky.x as f64,
                            g: sky.y as f64,
                            b: sky.z as f64,
                            a: 1.0,
                        }),
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
            if let Some(outline) = self.outline.as_ref() {
                outline.draw(&mut rpass);
            }
        }

        // Post pass: blit HDR target into the swapchain view.
        let post_writes = self.timer.as_ref().and_then(|t| t.render_writes(PASS_POST));
        if let Some(post) = self.post.as_ref() {
            post.draw(&mut encoder, &view, post_writes);
        }

        // Resolve GPU timestamps into the readback buffer.
        if let Some(timer) = &self.timer {
            timer.resolve(&mut encoder);
        }

        // Capture all values before the egui closure to avoid borrowing self.
        let fps = self.fps_smoothed;
        let pass_ms: Vec<(&str, f32)> = self
            .timer
            .as_ref()
            .map(|t| t.labels().iter().copied().zip(t.pass_ms.iter().copied()).collect())
            .unwrap_or_default();
        let gpu_total = self.timer.as_ref().map(|t| t.total_ms()).unwrap_or(0.0);
        let cam = self.camera.position;
        let eye_cell = glam::IVec3::new(
            cam.x.floor() as i32,
            cam.y.floor() as i32,
            cam.z.floor() as i32,
        );
        let light_label = {
            use crate::world::light::LightChannel;
            match (
                self.world.light(LightChannel::Sky, eye_cell),
                self.world.light(LightChannel::Block, eye_cell),
            ) {
                (Some(s), Some(b)) => format!("sky {s} / block {b}"),
                _ => "—".to_string(),
            }
        };
        let mode = match self.player.mode {
            crate::game::player::MoveMode::Walk => "walk",
            crate::game::player::MoveMode::Fly => "fly",
        };
        let target_label = self
            .target
            .map(|h| format!("{} {} {}", h.block.x, h.block.y, h.block.z))
            .unwrap_or_else(|| "—".to_string());
        let cols = self.stats.columns_ready;
        let visible = self.stats.visible_sections;
        let resident = self.stats.resident_sections;
        let quads = self.stats.drawn_quads;
        let cave_culled = self.stats.cave_culled;
        let cave_cull_on = self.cave_culling;
        let gen_q = self.jobs.gen_in_flight;
        let mesh_q = self.jobs.mesh_in_flight;
        let uploads = self.upload_queue.len();
        let (arena_used, arena_cap) =
            self.terrain.as_ref().map(|t| t.arena_usage()).unwrap_or((0, 1));
        let hotbar_slots = self.hotbar.slots;
        let hotbar_selected = self.hotbar.selected;
        let hud_visible = self.hud_visible;
        let paused = !self.cursor_grabbed;
        let day_label = format!(
            "{:02}:{:02} (×{:.2})",
            ((self.day.time * 24.0 + 6.0) % 24.0) as u32,
            ((self.day.time * 24.0 * 60.0) % 60.0) as u32,
            self.day.day_factor()
        );

        // Draw egui overlay: crosshair + hotbar always; Debug HUD only when F3.
        let egui_cmds = if let Some(egui) = &mut self.egui {
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
                    if paused {
                        crate::render::game_ui::draw_pause_overlay(ctx);
                    } else {
                        crate::render::game_ui::draw_crosshair(ctx);
                    }
                    crate::render::game_ui::draw_hotbar(ctx, &hotbar_slots, hotbar_selected);
                    if hud_visible {
                        egui::Window::new("Debug HUD")
                            .resizable(false)
                            .collapsible(false)
                            .show(ctx, |ui| {
                                ui.label(format!("FPS:      {fps:.1}"));
                                ui.label(format!("GPU ms:   {gpu_total:.2}"));
                                for (label, ms) in &pass_ms {
                                    ui.label(format!("  {label:<9} {ms:.2}"));
                                }
                                ui.label(format!("Pos:      {:.0} {:.0} {:.0}", cam.x, cam.y, cam.z));
                                ui.label(format!("Mode:     {mode}"));
                                ui.label(format!("Target:   {target_label}"));
                                ui.label(format!("Light:    {light_label}"));
                                ui.label(format!("Time:     {day_label}"));
                                ui.label(format!("Columns:  {cols}"));
                                ui.label(format!(
                                    "Sections: {visible}/{resident} drawn/resident (cave-culled {cave_culled})"
                                ));
                                ui.label(format!("CaveCull: {}", if cave_cull_on { "on (V)" } else { "OFF (V)" }));
                                ui.label(format!("Quads:    {quads}"));
                                ui.label(format!("Jobs:     gen {gen_q}  mesh {mesh_q}  upload {uploads}"));
                                ui.label(format!(
                                    "Arena:    {:.1}/{:.0} MiB",
                                    arena_used as f32 * 8.0 / (1 << 20) as f32,
                                    arena_cap as f32 * 8.0 / (1 << 20) as f32
                                ));
                            });
                    }
                },
            );
            Some(cmds)
        } else {
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
        self.input.end_frame();
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

        // Watchers first: their baseline mtime must predate the source reads,
        // so a save landing in between is detected as a change instead of missed.
        let mut shaders = crate::render::hot_reload::ShaderSet::new();
        shaders.watch("terrain", shader_path("terrain.wgsl"));
        shaders.watch("outline", shader_path("outline.wgsl"));
        shaders.watch("post", shader_path("post.wgsl"));
        shaders.watch("shadow", shader_path("shadow.wgsl"));
        self.shaders = Some(shaders);

        let size = window.inner_size();
        let targets =
            crate::render::targets::RenderTargets::new(&gpu.device, size.width, size.height);

        let hdr_format = crate::render::targets::HDR_FORMAT;
        let terrain_src =
            std::fs::read_to_string(shader_path("terrain.wgsl")).expect("terrain.wgsl missing");
        self.terrain = Some(TerrainRenderer::new(&gpu.device, hdr_format, &terrain_src));
        let shadow_src =
            std::fs::read_to_string(shader_path("shadow.wgsl")).expect("shadow.wgsl missing");
        let terrain_ref = self.terrain.as_ref().unwrap();
        self.shadow = Some(crate::render::shadow::ShadowRenderer::new(
            &gpu.device,
            terrain_ref.quads_layout(),
            &shadow_src,
        ));

        let outline_src =
            std::fs::read_to_string(shader_path("outline.wgsl")).expect("outline.wgsl missing");
        self.outline = Some(crate::render::outline::OutlineRenderer::new(
            &gpu.device,
            hdr_format,
            &outline_src,
        ));

        let post_src =
            std::fs::read_to_string(shader_path("post.wgsl")).expect("post.wgsl missing");
        self.post = Some(crate::render::post::PostPass::new(
            &gpu.device,
            gpu.config.format,
            &targets.hdr_view,
            &post_src,
        ));
        self.targets = Some(targets);

        // Initialize egui and GPU timer.
        self.egui = Some(EguiLayer::new(&gpu.device, gpu.config.format, &window));
        self.timer = Some(GpuTimer::new(&gpu.device, PASS_LABELS));

        self.gpu = Some(gpu);
        self.window = Some(window);
        self.set_cursor_grab(true);
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
                    self.targets = Some(crate::render::targets::RenderTargets::new(
                        &gpu.device,
                        size.width,
                        size.height,
                    ));
                    if let (Some(post), Some(targets)) =
                        (self.post.as_mut(), self.targets.as_ref())
                    {
                        post.set_input(&gpu.device, &targets.hdr_view);
                    }
                }
                return;
            }
            WindowEvent::KeyboardInput { event: key, .. }
                if key.state.is_pressed() && !key.repeat =>
            {
                match key.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => {
                        self.set_cursor_grab(false);
                        return;
                    }
                    // H mirrors F3: macOS routes bare function keys to system
                    // shortcuts (F3 = Mission Control), so F3 needs Fn held.
                    PhysicalKey::Code(KeyCode::F3) | PhysicalKey::Code(KeyCode::KeyH) => {
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
            WindowEvent::MouseInput { state, button, .. } => {
                if !self.cursor_grabbed {
                    // Click-to-refocus: re-grab and swallow the click so it doesn't
                    // break a block.
                    if state.is_pressed() {
                        self.set_cursor_grab(true);
                    }
                    return;
                }
                let mapped = match button {
                    winit::event::MouseButton::Left => Some(crate::game::input::MouseButton::Left),
                    winit::event::MouseButton::Right => Some(crate::game::input::MouseButton::Right),
                    _ => None,
                };
                if let Some(b) = mapped {
                    self.input.set_mouse_button(b, state.is_pressed());
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.input.accumulate_scroll(steps);
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
