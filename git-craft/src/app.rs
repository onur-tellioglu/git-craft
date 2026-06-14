use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::bench::{BenchConfig, BenchRun};
use crate::game::camera::Camera;
use crate::game::input::InputState;
use crate::render::depth;
use crate::render::egui_layer::EguiLayer;
use crate::render::gpu::Gpu;
use crate::render::terrain::TerrainRenderer;
use crate::render::timestamps::GpuTimer;

/// GPU pass timing slots (spec §8). Order is frame order; indices are stable
/// within a task but renumbered as the frame graph grows through M5.
const PASS_LABELS: &[&str] = &[
    "luts",
    "shadow0",
    "shadow1",
    "shadow2",
    "main",
    "gtao",
    "volumetric",
    "composite",
    "water",
    "taa",
    "bloom",
    "exposure",
    "post",
];
const PASS_LUTS: usize = 0;
const PASS_SHADOW0: usize = 1;
const PASS_MAIN: usize = 4;
const PASS_GTAO: usize = 5;
const PASS_VOLUMETRIC: usize = 6;
const PASS_COMPOSITE: usize = 7;
const PASS_WATER: usize = 8;
const PASS_TAA: usize = 9;
const PASS_BLOOM: usize = 10;
const PASS_EXPOSURE: usize = 11;
const PASS_POST: usize = 12;

/// GTAO tuning (one place to tweak the look; see the M5b GTAO plan).
/// World-space sample radius in blocks (1 block = 1 m).
const GTAO_RADIUS: f32 = 1.5;
/// Occlusion intensity multiplier (higher = darker contact shadows).
const GTAO_INTENSITY: f32 = 1.4;
/// Horizon bias subtracted before accumulating, to suppress self-occlusion.
const GTAO_BIAS: f32 = 0.02;
/// Max screen-space march radius in half-res pixels (caps the sample spread).
const GTAO_MAX_RADIUS_PX: f32 = 48.0;
/// Contrast power applied to the final visibility.
const GTAO_POWER: f32 = 1.5;
/// Bilateral-blur edge-stop sigma, in NDC depth units: smaller = sharper edges.
const GTAO_BLUR_DEPTH_SIGMA: f32 = 0.0015;

/// Volumetric fog/god-ray tuning (one place to tweak the look; see the M5c plan).
/// Base scattering coefficient per meter (scaled by the height/haze profile).
const VOL_DENSITY: f32 = 0.006;
/// Uniform haze floor added to the height term (keeps far air slightly milky).
const VOL_HAZE: f32 = 0.1;
/// Fog base altitude (blocks): fog is full-strength at/below this y, thinning above.
const VOL_FOG_Y0: f32 = 64.0;
/// Fog scale height (blocks): larger = fog reaches higher.
const VOL_FOG_H: f32 = 30.0;
/// Absorption as a fraction of scattering (mostly-scattering fog stays bright).
const VOL_ABSORB: f32 = 0.1;
/// Henyey-Greenstein anisotropy: >0 forward-scatters into a sun halo.
const VOL_HG_G: f32 = 0.6;
/// Isotropic ambient in-scatter strength (sky color fills shadowed fog).
const VOL_AMBIENT: f32 = 0.4;
/// Temporal reprojection blend (M5c Task 3): fraction of the new estimate kept.
const VOL_TAA_ALPHA: f32 = 0.05;

/// Water tuning (M5d). Tint color the refracted scene fades toward with depth.
const WATER_TINT: [f32; 3] = [0.04, 0.13, 0.18];
/// Fog density per meter of water column (higher = murkier, hides the seafloor sooner).
const WATER_FOG_DENSITY: f32 = 0.06;
/// Fresnel reflectance at normal incidence (water ≈ 0.02).
const WATER_FRESNEL_F0: f32 = 0.02;
/// Reflection strength multiplier on the fresnel term.
const WATER_REFLECTION: f32 = 1.0;
/// Screen-space refraction offset scale (how far the normal bends the lookup).
const WATER_REFRACTION: f32 = 0.03;

/// Candidate `assets/shaders` directories, most-preferred first.
///
/// A `cargo` build finds the live source tree via `CARGO_MANIFEST_DIR`, which keeps
/// hot-reload pointed at the editable shaders. A shipped binary's manifest path does
/// not exist on the user's machine, so resolution falls through to the `assets/`
/// directory bundled next to the executable.
fn shader_dir_candidates() -> Vec<PathBuf> {
    let mut bases = vec![PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/shaders"
    ))];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        bases.push(dir.join("assets/shaders"));
    }
    bases
}

/// First candidate that is an existing directory. Pure helper so resolution is testable.
fn pick_existing_dir(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|d| d.is_dir()).cloned()
}

fn shader_path(name: &str) -> String {
    let candidates = shader_dir_candidates();
    let dir = pick_existing_dir(&candidates).unwrap_or_else(|| candidates[0].clone());
    dir.join(name).to_string_lossy().into_owned()
}

/// Offscreen target size for a swapchain size and render scale, clamped to >= 1.
fn render_dims(width: u32, height: u32, scale: f32) -> (u32, u32) {
    (
        ((width as f32 * scale) as u32).max(1),
        ((height as f32 * scale) as u32).max(1),
    )
}

/// Cycle the render scale: 1.0 → 0.75 → 0.5 → 1.0 (the §11 fallback ladder).
fn next_render_scale(scale: f32) -> f32 {
    if scale > 0.9 {
        0.75
    } else if scale > 0.6 {
        0.5
    } else {
        1.0
    }
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

/// Bench-mode constants (see `src/bench.rs`). A fixed elevated vantage that
/// rotates in place over the recorded window: the loaded set stays resident
/// (no streaming churn) while frustum culling is exercised in every direction.
const BENCH_PITCH: f32 = -0.45; // ~-26°, tilted down over the terrain
const BENCH_TARGET_FPS: f32 = 120.0;
const BENCH_WINDOW: (u32, u32) = (1280, 720);
/// Frozen day-cycle phase during the bench (noon: full sun + shadows).
const BENCH_NOON: f32 = 0.25;

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
    depth_texture: Option<wgpu::Texture>,
    /// DepthOnly-aspect view of the depth texture — bound to the TAA shader for textureLoad.
    depth_sample_view: Option<wgpu::TextureView>,
    input: InputState,
    camera: Camera,
    last_frame: std::time::Instant,
    terrain: Option<TerrainRenderer>,
    shadow: Option<crate::render::shadow::ShadowRenderer>,
    sky_luts: Option<crate::render::atmosphere::SkyLuts>,
    sky_pass: Option<crate::render::atmosphere::SkyPass>,
    bloom: Option<crate::render::bloom::BloomPass>,
    exposure: Option<crate::render::exposure::ExposurePass>,
    shaders: Option<crate::render::hot_reload::ShaderSet>,
    targets: Option<crate::render::targets::RenderTargets>,
    post: Option<crate::render::post::PostPass>,
    taa: Option<crate::render::taa::TaaPass>,
    gtao: Option<crate::render::gtao::GtaoPass>,
    blur: Option<crate::render::gtao::BlurPass>,
    composite: Option<crate::render::gtao::CompositePass>,
    volumetric: Option<crate::render::volumetric::VolumetricPass>,
    water: Option<crate::render::water::WaterRenderer>,
    /// Previous frame's UNJITTERED view_proj (for TAA reprojection).
    prev_view_proj: glam::Mat4,
    /// Ping-pong index: which history_views slot is read this frame (0 or 1).
    taa_history_idx: usize,
    /// Running frame counter; incremented each render() call.
    frame_index: u64,
    /// 0.0 on first frame and after resize; 1.0 once history is valid.
    taa_valid: f32,
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
    atmosphere: crate::render::atmosphere::Atmosphere,
    world: crate::world::chunks::ChunkMap,
    worldgen: crate::world::r#gen::WorldGen,
    jobs: crate::world::jobs::Jobs,
    #[allow(clippy::type_complexity)]
    upload_queue: VecDeque<(
        crate::world::chunks::SectionPos,
        Vec<crate::mesh::quad::PackedQuad>,
        Vec<crate::mesh::quad::PackedQuad>,
    )>,
    /// Latest mesh-job version per section. Two jobs for one section can
    /// finish out of order; only a result matching the current version may
    /// be uploaded, or a stale snapshot would overwrite the fresh mesh.
    mesh_versions: HashMap<crate::world::chunks::SectionPos, u64>,
    /// Face-connectivity mask per meshed section (cave culling, spec §6).
    visibility_masks: HashMap<crate::world::chunks::SectionPos, u16>,
    cave_culling: bool,
    gtao_debug: bool,
    vol_debug: bool,
    /// Offscreen render scale (§11 safety valve): all HDR passes run at
    /// scale×swapchain; the post pass upscales by sampling the resolved target.
    render_scale: f32,
    stats: FrameStats,
    /// `Some` in `--bench` mode: drives the deterministic flythrough and
    /// records frame-time / GPU-time percentiles. `None` for normal play.
    bench: Option<BenchRun>,
    /// Set by the bench when it finishes; `about_to_wait` exits the loop.
    should_exit: bool,
    /// Player edits are persisted to region files on disk via a background
    /// worker (no main-thread I/O). Columns the player has touched are tracked
    /// and saved when they unload or the app exits.
    /// `None` in bench mode so benchmark flights do not pollute `saves/region/`
    /// or skew reproducibility.
    persistence: Option<crate::world::persistence::Persistence>,
    /// Columns with a payload on disk: stream them in by loading, not generating.
    saved_columns: HashSet<crate::world::chunks::ColumnPos>,
    /// Columns edited since they were last saved (i.e. since load/eviction).
    edited_columns: HashSet<crate::world::chunks::ColumnPos>,
}

impl App {
    pub fn new(instance: wgpu::Instance, bench_cfg: Option<BenchConfig>) -> Self {
        // Open the on-disk world and learn which columns are already saved.
        // Persistence is skipped in bench mode so benchmark flights do not write to
        // saves/region/ and skew results; see step 4 integration fix.
        let (persistence, saved_columns) = if bench_cfg.is_some() {
            (None, HashSet::new())
        } else {
            let (p, cols) =
                crate::world::persistence::Persistence::new(PathBuf::from("saves").join("region"));
            (Some(p), cols)
        };
        Self {
            instance: Some(instance),
            window: None,
            gpu: None,
            depth_view: None,
            depth_texture: None,
            depth_sample_view: None,
            input: InputState::default(),
            camera: Camera::new(glam::Vec3::new(16.0, 140.0, 16.0)),
            last_frame: std::time::Instant::now(),
            terrain: None,
            shadow: None,
            sky_luts: None,
            sky_pass: None,
            bloom: None,
            exposure: None,
            shaders: None,
            targets: None,
            post: None,
            taa: None,
            gtao: None,
            blur: None,
            composite: None,
            volumetric: None,
            water: None,
            prev_view_proj: glam::Mat4::IDENTITY,
            taa_history_idx: 0,
            frame_index: 0,
            taa_valid: 0.0,
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
            atmosphere: crate::render::atmosphere::Atmosphere::default(),
            world: crate::world::chunks::ChunkMap::default(),
            worldgen: crate::world::r#gen::WorldGen::new(SEED),
            jobs: crate::world::jobs::Jobs::new(),
            upload_queue: VecDeque::new(),
            mesh_versions: HashMap::new(),
            visibility_masks: HashMap::new(),
            cave_culling: true,
            gtao_debug: false,
            vol_debug: false,
            render_scale: 1.0,
            stats: FrameStats::default(),
            bench: bench_cfg.map(BenchRun::new),
            should_exit: false,
            persistence,
            saved_columns,
            edited_columns: HashSet::new(),
        }
    }

    /// Recreate the offscreen depth + render targets at `rw×rh` and rebuild every
    /// pass bind group that samples them. Shared by window resize and render-scale
    /// changes; the swapchain itself is resized separately (always full size).
    fn rebuild_offscreen(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, rw: u32, rh: u32) {
        let (depth_tex, depth_view) = depth::create_depth_texture(device, rw, rh);
        let depth_sample_view = depth_tex.create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        self.depth_texture = Some(depth_tex);
        self.depth_view = Some(depth_view);
        self.depth_sample_view = Some(depth_sample_view);
        self.targets = Some(crate::render::targets::RenderTargets::new(device, rw, rh));
        // History is now stale; discard it on the next frame.
        self.taa_valid = 0.0;
        if let (Some(taa), Some(targets), Some(depth_sv)) = (
            self.taa.as_mut(),
            self.targets.as_ref(),
            self.depth_sample_view.as_ref(),
        ) {
            taa.rebuild_bind_groups(device, targets, depth_sv);
        }
        if let (Some(gtao), Some(targets), Some(depth_sv)) = (
            self.gtao.as_mut(),
            self.targets.as_ref(),
            self.depth_sample_view.as_ref(),
        ) {
            gtao.rebuild_bind_group(device, depth_sv, &targets.gbuf_view);
        }
        if let (Some(blur), Some(targets), Some(depth_sv)) = (
            self.blur.as_mut(),
            self.targets.as_ref(),
            self.depth_sample_view.as_ref(),
        ) {
            blur.rebuild_bind_group(device, &targets.ao_raw_view, depth_sv);
        }
        if let (Some(composite), Some(targets), Some(vol), Some(depth_sv)) = (
            self.composite.as_mut(),
            self.targets.as_ref(),
            self.volumetric.as_ref(),
            self.depth_sample_view.as_ref(),
        ) {
            composite.rebuild_bind_group(
                device,
                &targets.hdr_view,
                &targets.gbuf_view,
                &targets.ao_blur_view,
                vol.integrated_view(),
                depth_sv,
            );
        }
        if let (Some(bloom), Some(targets)) = (self.bloom.as_mut(), self.targets.as_ref()) {
            bloom.set_targets(device, queue, targets);
        }
        if let (Some(exposure), Some(targets)) = (self.exposure.as_mut(), self.targets.as_ref()) {
            // Read the TAA-resolved stable frame, not raw HDR.
            exposure.set_input(device, &targets.resolved_view);
        }
        if let (Some(post), Some(targets)) = (self.post.as_mut(), self.targets.as_ref())
            && let Some(exposure_buf) = self.exposure.as_ref().map(|e| e.result_buffer())
        {
            post.set_input(
                device,
                &targets.resolved_view,
                &targets.bloom_views[0],
                exposure_buf,
            );
        }
        if let (Some(water), Some(targets), Some(depth_sv)) = (
            self.water.as_mut(),
            self.targets.as_ref(),
            self.depth_sample_view.as_ref(),
        ) {
            water.rebuild_bind_group(device, &targets.scene_color_view, depth_sv);
        }
    }

    /// Grab (lock + hide) or release the cursor. Input state is cleared on
    /// both transitions so half-held keys/buttons don't leak across.
    fn set_cursor_grab(&mut self, grab: bool) {
        let Some(window) = &self.window else { return };
        if grab {
            if window
                .set_cursor_grab(winit::window::CursorGrabMode::Locked)
                .is_err()
            {
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
                self.edited_columns
                    .insert(crate::world::chunks::block_to_column(
                        hit.block.x,
                        hit.block.z,
                    ));
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
            let free = self
                .world
                .block_at(cell)
                .is_some_and(|b| b == AIR || b == WATER);
            if free && !self.player.aabb().intersects_cell(cell) {
                if self.world.set_block(cell, self.hotbar.selected_block()) {
                    crate::world::light_engine::on_block_changed(&mut self.world, cell);
                    self.edited_columns
                        .insert(crate::world::chunks::block_to_column(cell.x, cell.z));
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
        use crate::world::chunks::{ColumnPos, SectionPos, columns_in_radius};
        use crate::world::jobs::JobResult;
        let Some(gpu) = self.gpu.as_ref() else { return };

        let center = ColumnPos {
            x: (self.camera.position.x as i32).div_euclid(32),
            z: (self.camera.position.z as i32).div_euclid(32),
        };

        // 1. Drain finished jobs.
        for result in self.jobs.drain() {
            match result {
                JobResult::Generated {
                    pos,
                    data,
                    light,
                    writes,
                } => {
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
                JobResult::Meshed {
                    pos,
                    version,
                    opaque,
                    water,
                    visibility,
                } => {
                    let current = self.mesh_versions.get(&pos).copied().unwrap_or(0);
                    if version == current && self.world.ready(pos.column()).is_some() {
                        self.visibility_masks.insert(pos, visibility);
                        self.upload_queue.push_back((pos, opaque, water));
                    }
                    // version < current: a newer job is in flight (or already
                    // landed) for this section — stale snapshot, drop it.
                }
            }
        }

        // 1b. Drain finished disk loads and save acknowledgements. A loaded
        // column takes the same insert + seam-heal path as a generated one
        // (light recomputed by the worker).
        if let Some(p) = self.persistence.as_mut() {
            for loaded in p.drain_loaded() {
                use crate::world::persistence::Loaded;
                match loaded {
                    Loaded::Column { pos, data, light } => {
                        let d2 = (pos.x - center.x).pow(2) + (pos.z - center.z).pow(2);
                        if d2 > UNLOAD_RADIUS * UNLOAD_RADIUS {
                            continue; // moved away; the unload pass reaps the slot
                        }
                        let touched = self.world.insert_generated(pos, data, *light, Vec::new());
                        crate::world::light_engine::seed_column_borders(&mut self.world, pos);
                        for wp in touched {
                            crate::world::light_engine::on_block_changed(&mut self.world, wp);
                        }
                    }
                    Loaded::Failed { pos } => {
                        // Corrupt/missing payload: drop it from the saved set and
                        // regenerate, unless the player has already moved on.
                        self.saved_columns.remove(&pos);
                        let d2 = (pos.x - center.x).pow(2) + (pos.z - center.z).pow(2);
                        if d2 <= UNLOAD_RADIUS * UNLOAD_RADIUS && self.world.contains(pos) {
                            self.jobs.spawn_gen(self.worldgen.clone(), pos);
                        }
                    }
                    // Save acknowledgements: only mark a column as saved once the
                    // worker confirms the write succeeded, so a disk error is never
                    // silently treated as a successful save.
                    Loaded::SaveOk { pos } => {
                        self.saved_columns.insert(pos);
                    }
                    Loaded::SaveFailed { pos } => {
                        log::error!(
                            "column {pos:?} could not be saved to disk — player edits may be lost"
                        );
                        // The column data has already been unloaded; we cannot
                        // retry the save in this session, but we keep it out of
                        // saved_columns so the next session does not skip generation.
                    }
                }
            }
        }

        // 1c. Persist edited columns about to leave the keep radius, before the
        // unload pass drops their data.
        let unload_r2 = UNLOAD_RADIUS * UNLOAD_RADIUS;
        let leaving: Vec<ColumnPos> = self
            .edited_columns
            .iter()
            .filter(|c| (c.x - center.x).pow(2) + (c.z - center.z).pow(2) > unload_r2)
            .copied()
            .collect();
        for col in leaving {
            if let (Some(p), Some(c)) = (self.persistence.as_ref(), self.world.ready(col)) {
                p.request_save(col, c.sections.to_vec());
                // Do NOT insert into saved_columns here: wait for SaveOk so a
                // disk error is not silently treated as a successful save.
            }
            self.edited_columns.remove(&col);
        }

        // 2. Unload far columns and free their GPU meshes + version entries.
        if let Some(terrain) = self.terrain.as_mut() {
            for pos in self.world.unload_outside(center, UNLOAD_RADIUS) {
                for y in 0..8 {
                    let section = SectionPos {
                        x: pos.x,
                        y,
                        z: pos.z,
                    };
                    terrain.remove_section(section);
                    self.mesh_versions.remove(&section);
                    self.visibility_masks.remove(&section);
                }
            }
        }

        // 3. Request streaming for nearby missing columns, nearest first.
        // Saved columns load from disk; the rest generate. Both draw on one
        // in-flight budget so loads can't starve generation or vice versa.
        let persist_in_flight = self.persistence.as_ref().map_or(0, |p| p.load_in_flight);
        if self.jobs.gen_in_flight + persist_in_flight < MAX_GEN_IN_FLIGHT {
            for col in columns_in_radius(center, LOAD_RADIUS) {
                let persist_in_flight = self.persistence.as_ref().map_or(0, |p| p.load_in_flight);
                if self.jobs.gen_in_flight + persist_in_flight >= MAX_GEN_IN_FLIGHT {
                    break;
                }
                if !self.world.contains(col) {
                    self.world.mark_generating(col);
                    if self.saved_columns.contains(&col) {
                        if let Some(p) = self.persistence.as_mut() {
                            p.request_load(col);
                        }
                    } else {
                        self.jobs.spawn_gen(self.worldgen.clone(), col);
                    }
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
                    let pos = SectionPos {
                        x: col.x,
                        y: sy as i32,
                        z: col.z,
                    };
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
                let Some((pos, opaque, water)) = self.upload_queue.pop_front() else {
                    break;
                };
                if self.world.ready(pos.column()).is_none() {
                    continue; // unloaded while queued
                }
                terrain.upload_section(&gpu.queue, pos, &opaque, &water);
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
                    let col = ColumnPos {
                        x: pos.x + dx,
                        z: pos.z + dz,
                    };
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
                    "sky_luts" => {
                        if let Some(l) = self.sky_luts.as_mut() {
                            l.swap_shader(&gpu.device, &source);
                        }
                    }
                    "sky" => {
                        if let (Some(s), Some(t)) = (self.sky_pass.as_mut(), self.terrain.as_ref())
                        {
                            s.swap_shader(&gpu.device, t.camera_layout(), &source);
                        }
                    }
                    "bloom" => {
                        if let Some(b) = self.bloom.as_mut() {
                            b.swap_shader(&gpu.device, &source);
                        }
                    }
                    "exposure" => {
                        if let Some(e) = self.exposure.as_mut() {
                            e.swap_shader(&gpu.device, &source);
                        }
                    }
                    "taa" => {
                        if let Some(t) = self.taa.as_mut() {
                            t.swap_shader(&gpu.device, &source);
                        }
                    }
                    "gtao" => {
                        if let Some(g) = self.gtao.as_mut() {
                            g.swap_shader(&gpu.device, &source);
                        }
                    }
                    "gtao_blur" => {
                        if let Some(b) = self.blur.as_mut() {
                            b.swap_shader(&gpu.device, &source);
                        }
                    }
                    "composite" => {
                        if let Some(c) = self.composite.as_mut() {
                            c.swap_shader(&gpu.device, &source);
                        }
                    }
                    "volumetric" => {
                        if let Some(v) = self.volumetric.as_mut() {
                            v.swap_shader(&gpu.device, &source);
                        }
                    }
                    "water" => {
                        if let (Some(w), Some(t)) = (self.water.as_mut(), self.terrain.as_ref()) {
                            w.swap_shader(
                                &gpu.device,
                                t.camera_layout(),
                                t.quads_layout(),
                                &source,
                            );
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

        if let Some(run) = self.bench.as_ref() {
            // Deterministic flythrough: a fixed vantage that orbits in place,
            // with the day cycle frozen at noon. Overrides all gameplay input;
            // the world streams around this point during warmup.
            self.camera.position = glam::Vec3::new(16.0, 140.0, 16.0);
            self.camera.yaw = crate::bench::bench_yaw(run.recorded(), run.frames());
            self.camera.pitch = BENCH_PITCH;
            self.day.time = BENCH_NOON;
        } else {
            // Advance day/night cycle unconditionally — time flows even while paused.
            self.day.advance(dt);
        }

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
            if self.input.key_pressed(KeyCode::KeyG) {
                self.gtao_debug = !self.gtao_debug;
            }
            // B (V is taken by cave-culling): show the raw volumetric in-scatter.
            if self.input.key_pressed(KeyCode::KeyB) {
                self.vol_debug = !self.vol_debug;
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
                KeyCode::Digit1,
                KeyCode::Digit2,
                KeyCode::Digit3,
                KeyCode::Digit4,
                KeyCode::Digit5,
                KeyCode::Digit6,
                KeyCode::Digit7,
                KeyCode::Digit8,
                KeyCode::Digit9,
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
                self.player
                    .update(&self.input, self.camera.yaw, dt, &is_solid, &is_water);
            }
            self.camera.position = self.player.eye();

            self.update_interaction(dt);
        } else {
            // No targeted block while paused: hides the outline.
            self.target = None;
        }

        // World streaming: gen/mesh/upload jobs.
        self.update_world();

        // Bench state machine: warm up until streaming goes quiet, then record
        // one (cpu_ms, gpu_ms) sample per frame. GPU ms is the previous frame's
        // resolved timestamp readback (vsync-independent); cpu_ms is frame dt.
        if self.bench.is_some() {
            let idle = self.jobs.gen_in_flight == 0
                && self.jobs.mesh_in_flight == 0
                && self.upload_queue.is_empty()
                && self.stats.columns_ready > 0;
            let gpu_ms = self.timer.as_ref().map(|t| t.total_ms()).unwrap_or(0.0);
            let ts_enabled = self.timer.as_ref().is_some_and(|t| t.timestamps_enabled());
            if let Some(run) = self.bench.as_mut() {
                if run.is_warming() {
                    run.warmup_step(idle);
                } else {
                    run.push(dt * 1000.0, gpu_ms);
                    if run.is_done() {
                        let cpu = run.cpu_summary().unwrap();
                        let gpu = run.gpu_summary().unwrap();
                        // Primary signal: were timestamp queries created?
                        // Secondary guard: reject an all-zero readback batch
                        // (every sample stalled), which would give a bogus GPU
                        // verdict of 0 ms. Warn so the user can diagnose it.
                        if ts_enabled && gpu.max == 0.0 {
                            log::warn!(
                                "TIMESTAMP_QUERY is enabled but all GPU readbacks \
                                 returned 0 — possible Metal driver stall; \
                                 falling back to CPU p99 for the verdict"
                            );
                        }
                        let timestamps = ts_enabled && gpu.max > 0.0;
                        let report = crate::bench::format_report(
                            &cpu,
                            &gpu,
                            BENCH_TARGET_FPS,
                            timestamps,
                            RENDER_RADIUS,
                        );
                        println!("{report}");
                        self.should_exit = true;
                    }
                }
            }
        }

        // Disjoint field borrows: terrain mutably for prepare, gpu mutably.
        let Some(depth_view_ref) = self.depth_view.as_ref() else {
            return;
        };
        let Some(gpu) = self.gpu.as_mut() else { return };

        let aspect = gpu.config.width as f32 / gpu.config.height as f32;
        let view_proj = self.camera.view_proj(aspect); // unjittered — used for cull/shadows/reprojection
        // Offscreen render resolution (render-scale safety valve); the swapchain
        // stays full size and the post pass upscales the resolved target.
        let (rw, rh) = render_dims(gpu.config.width, gpu.config.height, self.render_scale);

        // Compute the sub-pixel jitter for this frame.
        self.frame_index += 1;
        let (jx, jy) = crate::render::taa::jitter_offset(self.frame_index);
        let (w, h) = (rw as f32, rh as f32);
        // NDC sub-pixel offset (one pixel = 2/size in NDC).
        let jitter = glam::Mat4::from_translation(glam::vec3(2.0 * jx / w, 2.0 * jy / h, 0.0));
        let jittered_vp = jitter * view_proj;

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
            Some(crate::render::visibility::visible_set(
                cam_section,
                RENDER_RADIUS,
                |p| masks.get(&p).copied(),
            ))
        } else {
            None
        };

        let altitude_km = (self.camera.position.y / 1000.0).max(1e-4); // 1 block = 1 m
        let (light_dir, light_color, light_is_sun) = crate::render::atmosphere::dominant_light(
            &self.atmosphere,
            self.day.sun_dir(),
            altitude_km,
        );

        if let Some(terrain) = self.terrain.as_mut() {
            terrain.write_frame(
                &gpu.queue,
                &crate::render::terrain::FrameParams {
                    view_proj: jittered_vp, // jittered for main-pass rasterization
                    camera_pos: self.camera.position,
                    sky_color: self.day.sky_color(),
                    day_factor: self.day.day_factor(),
                    light_dir,
                    light_is_sun,
                    light_color,
                    viewport: (rw, rh),
                },
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

        if let Some(luts) = self.sky_luts.as_ref() {
            luts.prepare(
                &gpu.queue,
                &crate::render::atmosphere::AtmUniform {
                    inv_view_proj: view_proj.inverse().to_cols_array_2d(),
                    camera: [
                        self.camera.position.x,
                        self.camera.position.y,
                        self.camera.position.z,
                        altitude_km,
                    ],
                    sun: [
                        self.day.sun_dir().x,
                        self.day.sun_dir().y,
                        self.day.sun_dir().z,
                        0.0,
                    ],
                    sun_radiance: [
                        crate::render::atmosphere::SUN_RADIANCE.x,
                        crate::render::atmosphere::SUN_RADIANCE.y,
                        crate::render::atmosphere::SUN_RADIANCE.z,
                        0.0,
                    ],
                },
            );
        }
        if let Some(exposure) = self.exposure.as_ref() {
            exposure.prepare(&gpu.queue, dt);
        }

        // Water uniform: tint/fog/fresnel tuning + an animation clock for ripples.
        if let Some(water) = self.water.as_ref() {
            water.prepare(
                &gpu.queue,
                &crate::render::water::WaterUniform {
                    tint: [
                        WATER_TINT[0],
                        WATER_TINT[1],
                        WATER_TINT[2],
                        WATER_FOG_DENSITY,
                    ],
                    params: [
                        self.frame_index as f32 * 0.03,
                        WATER_FRESNEL_F0,
                        WATER_REFLECTION,
                        WATER_REFRACTION,
                    ],
                },
            );
        }

        // Prepare TAA uniform: jittered inv_view_proj, prev unjittered view_proj.
        if let Some(taa) = self.taa.as_ref() {
            let taa_uniform = crate::render::taa::TaaUniform {
                inv_view_proj: jittered_vp.inverse().to_cols_array_2d(),
                prev_view_proj: self.prev_view_proj.to_cols_array_2d(),
                params: [w, h, crate::render::taa::BLEND, self.taa_valid],
            };
            taa.prepare(&gpu.queue, &taa_uniform);
        }

        // Prepare GTAO uniform: jittered inv_view_proj for consistent depth reconstruction.
        if let Some(gtao) = self.gtao.as_ref() {
            let (hw, hh) = crate::render::gtao::half_res(rw, rh);
            gtao.prepare(
                &gpu.queue,
                &crate::render::gtao::GtaoUniform {
                    inv_view_proj: jittered_vp.inverse().to_cols_array_2d(),
                    params: [hw as f32, hh as f32, GTAO_RADIUS, self.frame_index as f32],
                    tune: [GTAO_INTENSITY, GTAO_BIAS, GTAO_MAX_RADIUS_PX, GTAO_POWER],
                },
            );
        }
        if let Some(blur) = self.blur.as_ref() {
            let (hw, hh) = crate::render::gtao::half_res(rw, rh);
            blur.prepare(
                &gpu.queue,
                [hw as f32, hh as f32, GTAO_BLUR_DEPTH_SIGMA, 0.0],
            );
        }
        if let Some(composite) = self.composite.as_ref() {
            use crate::render::volumetric::{VOL_D, VOL_FAR, VOL_NEAR};
            composite.prepare(
                &gpu.queue,
                &crate::render::gtao::CompUniform {
                    flags: [
                        if self.gtao_debug { 1.0 } else { 0.0 },
                        if self.vol_debug { 1.0 } else { 0.0 },
                        0.0,
                        0.0,
                    ],
                    inv_view_proj: jittered_vp.inverse().to_cols_array_2d(),
                    camera: [
                        self.camera.position.x,
                        self.camera.position.y,
                        self.camera.position.z,
                        0.0,
                    ],
                    vol_params: [VOL_NEAR, VOL_FAR, VOL_D as f32, 0.0],
                },
            );
        }

        // Volumetric froxel grid: world-space, so it uses the UNJITTERED VP
        // (the grid stays jitter-free like the sky/aerial LUTs; the composite
        // pass samples it with jittered depth for the per-pixel world position).
        if let Some(vol) = self.volumetric.as_ref() {
            let sky = self.day.sky_color();
            vol.prepare(
                &gpu.queue,
                &crate::render::volumetric::VolUniform {
                    inv_view_proj: view_proj.inverse().to_cols_array_2d(),
                    prev_view_proj: self.prev_view_proj.to_cols_array_2d(),
                    camera: [
                        self.camera.position.x,
                        self.camera.position.y,
                        self.camera.position.z,
                        self.frame_index as f32,
                    ],
                    sun: [
                        light_dir.x,
                        light_dir.y,
                        light_dir.z,
                        if light_is_sun { 1.0 } else { 0.0 },
                    ],
                    sun_color: [light_color.x, light_color.y, light_color.z, 0.0],
                    sky: [sky.x, sky.y, sky.z, self.taa_valid],
                    fog: [VOL_DENSITY, VOL_HAZE, VOL_FOG_Y0, VOL_FOG_H],
                    tune: [VOL_ABSORB, VOL_HG_G, VOL_AMBIENT, VOL_TAA_ALPHA],
                },
            );
        }

        let Some(frame) = gpu.acquire() else {
            // Press edges were consumed by this frame's logic above; clear them
            // even when the swapchain frame is dropped, or a click would fire
            // a second edit on the next frame.
            self.input.end_frame();
            return;
        };
        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        let hdr_view = &targets.hdr_view;
        let gbuf_view = &targets.gbuf_view;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        let luts_writes = self
            .timer
            .as_ref()
            .and_then(|t| t.compute_writes(PASS_LUTS));
        if let Some(luts) = self.sky_luts.as_mut() {
            luts.encode(&mut encoder, luts_writes);
        }

        if let (Some(shadow), Some(terrain)) = (self.shadow.as_mut(), self.terrain.as_ref()) {
            shadow.encode(&mut encoder, terrain, self.timer.as_ref(), PASS_SHADOW0);
        }

        // Capture timestamp_writes before the block to avoid borrow issues.
        let ts_writes = self.timer.as_ref().and_then(|t| t.render_writes(PASS_MAIN));

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: hdr_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: gbuf_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view_ref,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store, // TAA resolve samples depth for reprojection
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
            if let (Some(sky), Some(terrain)) = (self.sky_pass.as_ref(), self.terrain.as_ref()) {
                sky.draw(&mut rpass, terrain.camera_bind_group());
            }
            if let Some(outline) = self.outline.as_ref() {
                outline.draw(&mut rpass);
            }
        }

        // GTAO: half-res horizon AO from depth + g-buffer normals.
        let gtao_writes = self.timer.as_ref().and_then(|t| t.render_writes(PASS_GTAO));
        if let (Some(gtao), Some(targets)) = (self.gtao.as_ref(), self.targets.as_ref()) {
            gtao.encode(&mut encoder, &targets.ao_raw_view, gtao_writes);
        }

        // Blur: depth-aware bilateral blur of raw AO (untimed, shares GTAO budget).
        if let (Some(blur), Some(targets)) = (self.blur.as_ref(), self.targets.as_ref()) {
            blur.encode(&mut encoder, &targets.ao_blur_view, None);
        }

        // Volumetric froxel grid: in-scatter (CSM god rays + height fog) then
        // front-to-back integrate. Composite samples the integrated grid next.
        let vol_writes = self
            .timer
            .as_ref()
            .and_then(|t| t.compute_writes(PASS_VOLUMETRIC));
        if let Some(vol) = self.volumetric.as_ref() {
            vol.encode(&mut encoder, (self.frame_index & 1) as usize, vol_writes);
        }

        // Composite: apply blurred AO to the HDR ambient term; TAA reads composited_view.
        let comp_writes = self
            .timer
            .as_ref()
            .and_then(|t| t.render_writes(PASS_COMPOSITE));
        if let (Some(composite), Some(targets)) = (self.composite.as_ref(), self.targets.as_ref()) {
            composite.encode(&mut encoder, &targets.composited_view, comp_writes);
        }

        // Water transparent pass: snapshot the opaque/composited scene into
        // scene_color (the refraction + SSR source), then draw water surfaces
        // back into composited_view, upstream of TAA so the edges resolve.
        let water_writes = self
            .timer
            .as_ref()
            .and_then(|t| t.render_writes(PASS_WATER));
        if let (Some(water), Some(terrain), Some(targets)) = (
            self.water.as_ref(),
            self.terrain.as_ref(),
            self.targets.as_ref(),
        ) {
            encoder.copy_texture_to_texture(
                targets.composited_texture.as_image_copy(),
                targets.scene_color_texture.as_image_copy(),
                wgpu::Extent3d {
                    width: targets.width,
                    height: targets.height,
                    depth_or_array_layers: 1,
                },
            );
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.composited_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: water_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            water.draw(&mut rpass, terrain);
        }

        // TAA resolve: reproject + neighborhood clamp + blend; writes resolved_view.
        let taa_writes = self.timer.as_ref().and_then(|t| t.render_writes(PASS_TAA));
        if let (Some(taa), Some(targets)) = (self.taa.as_ref(), self.targets.as_ref()) {
            taa.encode(&mut encoder, targets, self.taa_history_idx, taa_writes);
        }

        // Bloom down/up chain: runs after TAA resolve, before post.
        if let (Some(bloom), Some(targets)) = (self.bloom.as_ref(), self.targets.as_ref()) {
            bloom.encode(&mut encoder, targets, self.timer.as_ref(), PASS_BLOOM);
        }

        // Exposure histogram + resolve: runs after bloom, before post.
        let exp_writes = self
            .timer
            .as_ref()
            .and_then(|t| t.compute_writes(PASS_EXPOSURE));
        if let Some(exposure) = self.exposure.as_ref() {
            exposure.encode(&mut encoder, rw, rh, exp_writes);
        }

        // Post pass: blit resolved TAA target (with bloom mixed in, exposure applied) into swapchain.
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
            .map(|t| {
                t.labels()
                    .iter()
                    .copied()
                    .zip(t.pass_ms.iter().copied())
                    .collect()
            })
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
        let (arena_used, arena_cap) = self
            .terrain
            .as_ref()
            .map(|t| t.arena_usage())
            .unwrap_or((0, 1));
        let hotbar_slots = self.hotbar.slots;
        let hotbar_selected = self.hotbar.selected;
        let hud_visible = self.hud_visible;
        let paused = !self.cursor_grabbed;
        let render_scale = self.render_scale;
        let (render_w, render_h) = render_dims(gpu.config.width, gpu.config.height, render_scale);
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
                                ui.label(format!("Scale:    {render_scale:.2} → {render_w}×{render_h} (R)"));
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

        // TAA bookkeeping after submit: store unjittered VP for next frame's reprojection.
        self.prev_view_proj = view_proj; // unjittered
        self.taa_history_idx ^= 1;
        self.taa_valid = 1.0;

        frame.present();
        self.input.end_frame();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return; // macOS can resume more than once; init exactly once
        }
        // Bench mode forces a fixed window size so the baseline is comparable
        // run-to-run, and freezes the day cycle at noon for stable lighting.
        let bench = self.bench.is_some();
        let mut attrs = Window::default_attributes().with_title("git-craft");
        if bench {
            attrs = attrs.with_title("git-craft (bench)").with_inner_size(
                winit::dpi::PhysicalSize::new(BENCH_WINDOW.0, BENCH_WINDOW.1),
            );
            self.day.time = BENCH_NOON;
        }
        let window = Arc::new(event_loop.create_window(attrs).unwrap());
        let instance = self
            .instance
            .take()
            .expect("resumed twice with GPU already built");
        let gpu = Gpu::new(&instance, window.clone(), bench);
        let size = window.inner_size();
        let (rw, rh) = render_dims(size.width, size.height, self.render_scale);
        let (depth_tex, depth_view) = depth::create_depth_texture(&gpu.device, rw, rh);
        let depth_sample_view = depth_tex.create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        self.depth_texture = Some(depth_tex);
        self.depth_view = Some(depth_view);
        self.depth_sample_view = Some(depth_sample_view);

        // Watchers first: their baseline mtime must predate the source reads,
        // so a save landing in between is detected as a change instead of missed.
        let mut shaders = crate::render::hot_reload::ShaderSet::new();
        shaders.watch("terrain", shader_path("terrain.wgsl"));
        shaders.watch("outline", shader_path("outline.wgsl"));
        shaders.watch("post", shader_path("post.wgsl"));
        shaders.watch("shadow", shader_path("shadow.wgsl"));
        shaders.watch("sky_luts", shader_path("sky_luts.wgsl"));
        shaders.watch("sky", shader_path("sky.wgsl"));
        shaders.watch("bloom", shader_path("bloom.wgsl"));
        shaders.watch("exposure", shader_path("exposure.wgsl"));
        shaders.watch("taa", shader_path("taa.wgsl"));
        shaders.watch("gtao", shader_path("gtao.wgsl"));
        shaders.watch("gtao_blur", shader_path("gtao_blur.wgsl"));
        shaders.watch("composite", shader_path("composite.wgsl"));
        shaders.watch("volumetric", shader_path("volumetric.wgsl"));
        shaders.watch("water", shader_path("water.wgsl"));
        self.shaders = Some(shaders);

        let size = window.inner_size();
        let (rw, rh) = render_dims(size.width, size.height, self.render_scale);
        let targets = crate::render::targets::RenderTargets::new(&gpu.device, rw, rh);

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

        // Wire shadow resources into terrain group 2 (after both exist).
        if let (Some(terrain), Some(shadow)) = (self.terrain.as_mut(), self.shadow.as_ref()) {
            terrain.attach_shadow(&gpu.device, shadow.uniform_buffer(), shadow.array_view());
        }

        // Volumetric froxel grid: samples the CSM for god rays. Owns its own
        // fixed-size 3D textures (resize never touches them), like the sky LUTs.
        let volumetric_src = std::fs::read_to_string(shader_path("volumetric.wgsl"))
            .expect("volumetric.wgsl missing");
        if let Some(shadow) = self.shadow.as_ref() {
            self.volumetric = Some(crate::render::volumetric::VolumetricPass::new(
                &gpu.device,
                shadow.uniform_buffer(),
                shadow.array_view(),
                &volumetric_src,
            ));
        }

        let outline_src =
            std::fs::read_to_string(shader_path("outline.wgsl")).expect("outline.wgsl missing");
        self.outline = Some(crate::render::outline::OutlineRenderer::new(
            &gpu.device,
            hdr_format,
            &outline_src,
        ));

        let bloom_src =
            std::fs::read_to_string(shader_path("bloom.wgsl")).expect("bloom.wgsl missing");
        self.bloom = Some(crate::render::bloom::BloomPass::new(
            &gpu.device,
            &gpu.queue,
            &targets,
            &bloom_src,
        ));

        let exposure_src =
            std::fs::read_to_string(shader_path("exposure.wgsl")).expect("exposure.wgsl missing");
        self.exposure = Some(crate::render::exposure::ExposurePass::new(
            &gpu.device,
            &targets.resolved_view, // reads the TAA-resolved stable frame
            &exposure_src,
        ));

        let post_src =
            std::fs::read_to_string(shader_path("post.wgsl")).expect("post.wgsl missing");
        let exposure_buf = self.exposure.as_ref().unwrap().result_buffer();
        self.post = Some(crate::render::post::PostPass::new(
            &gpu.device,
            gpu.config.format,
            &targets.resolved_view, // reads the TAA-resolved stable frame
            &targets.bloom_views[0],
            exposure_buf,
            &post_src,
        ));

        let taa_src = std::fs::read_to_string(shader_path("taa.wgsl")).expect("taa.wgsl missing");
        let depth_sample_view_ref = self.depth_sample_view.as_ref().unwrap();
        self.taa = Some(crate::render::taa::TaaPass::new(
            &gpu.device,
            &targets,
            depth_sample_view_ref,
            &taa_src,
        ));

        let gtao_src =
            std::fs::read_to_string(shader_path("gtao.wgsl")).expect("gtao.wgsl missing");
        self.gtao = Some(crate::render::gtao::GtaoPass::new(
            &gpu.device,
            depth_sample_view_ref,
            &targets.gbuf_view,
            &gtao_src,
        ));

        let blur_src =
            std::fs::read_to_string(shader_path("gtao_blur.wgsl")).expect("gtao_blur.wgsl missing");
        self.blur = Some(crate::render::gtao::BlurPass::new(
            &gpu.device,
            &targets.ao_raw_view,
            depth_sample_view_ref,
            &blur_src,
        ));

        let composite_src =
            std::fs::read_to_string(shader_path("composite.wgsl")).expect("composite.wgsl missing");
        let froxel_view = self.volumetric.as_ref().unwrap().integrated_view();
        self.composite = Some(crate::render::gtao::CompositePass::new(
            &gpu.device,
            &targets.hdr_view,
            &targets.gbuf_view,
            &targets.ao_blur_view,
            froxel_view,
            depth_sample_view_ref,
            &composite_src,
        ));

        self.targets = Some(targets);

        let luts_src =
            std::fs::read_to_string(shader_path("sky_luts.wgsl")).expect("sky_luts.wgsl missing");
        self.sky_luts = Some(crate::render::atmosphere::SkyLuts::new(
            &gpu.device,
            &luts_src,
        ));

        // Wire aerial LUT into terrain group 3.
        if let (Some(terrain), Some(luts)) = (self.terrain.as_mut(), self.sky_luts.as_ref()) {
            terrain.attach_aerial(&gpu.device, &luts.aerial_view);
        }

        let sky_src = std::fs::read_to_string(shader_path("sky.wgsl")).expect("sky.wgsl missing");
        let terrain_ref = self.terrain.as_ref().unwrap();
        let luts_ref = self.sky_luts.as_ref().unwrap();
        self.sky_pass = Some(crate::render::atmosphere::SkyPass::new(
            &gpu.device,
            terrain_ref.camera_layout(),
            &luts_ref.skyview_view,
            &sky_src,
        ));

        // Transparent water pass: reuses terrain's camera/quads bind groups and
        // the sky-view LUT; its scene-color + depth bind group is wired below.
        let water_src =
            std::fs::read_to_string(shader_path("water.wgsl")).expect("water.wgsl missing");
        let mut water = {
            let terrain_ref = self.terrain.as_ref().unwrap();
            let luts_ref = self.sky_luts.as_ref().unwrap();
            crate::render::water::WaterRenderer::new(
                &gpu.device,
                terrain_ref.camera_layout(),
                terrain_ref.quads_layout(),
                &luts_ref.skyview_view,
                &water_src,
            )
        };
        if let (Some(targets), Some(depth_sv)) =
            (self.targets.as_ref(), self.depth_sample_view.as_ref())
        {
            water.rebuild_bind_group(&gpu.device, &targets.scene_color_view, depth_sv);
        }
        self.water = Some(water);

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
                // Clone the Arc-backed device/queue so the offscreen rebuild can
                // borrow self mutably without holding a borrow on self.gpu.
                let handles = self.gpu.as_mut().map(|gpu| {
                    gpu.resize(size.width, size.height);
                    (gpu.device.clone(), gpu.queue.clone())
                });
                if let Some((device, queue)) = handles {
                    let (rw, rh) = render_dims(size.width, size.height, self.render_scale);
                    self.rebuild_offscreen(&device, &queue, rw, rh);
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
                    // R cycles the render scale (1.0 → 0.75 → 0.5): the §11
                    // performance safety valve. Rebuilds the offscreen chain.
                    PhysicalKey::Code(KeyCode::KeyR) => {
                        self.render_scale = next_render_scale(self.render_scale);
                        let handles = self.gpu.as_ref().map(|gpu| {
                            (
                                gpu.device.clone(),
                                gpu.queue.clone(),
                                render_dims(gpu.config.width, gpu.config.height, self.render_scale),
                            )
                        });
                        if let Some((device, queue, (rw, rh))) = handles {
                            self.rebuild_offscreen(&device, &queue, rw, rh);
                        }
                        return;
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // Feed egui next; if it consumes the event, don't propagate to game input.
        if let (Some(egui), Some(window)) = (&mut self.egui, &self.window)
            && egui.on_window_event(window, &event)
        {
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
                    winit::event::MouseButton::Right => {
                        Some(crate::game::input::MouseButton::Right)
                    }
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
        // The bench sets this once it has printed its report; `render` has no
        // access to the event loop, so the exit happens here.
        if self.should_exit {
            _el.exit();
            return;
        }
        if self.occluded {
            return;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// On quit, save every still-loaded edited column, then drain the worker's
    /// queued saves and join it so nothing is lost.
    fn exiting(&mut self, _el: &ActiveEventLoop) {
        let edited: Vec<crate::world::chunks::ColumnPos> = self.edited_columns.drain().collect();
        for col in edited {
            if let (Some(p), Some(c)) = (self.persistence.as_ref(), self.world.ready(col)) {
                p.request_save(col, c.sections.to_vec());
                // Save acks are flushed by shutdown() (the FIFO channel
                // processes all queued saves before Shutdown reaches the
                // worker). saved_columns is not used after exit.
            }
        }
        if let Some(p) = self.persistence.as_mut() {
            p.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pick_existing_dir;
    use std::path::PathBuf;

    /// Build a unique temp base dir for this test so runs don't collide.
    fn temp_base(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gitcraft_{}_{}", tag, std::process::id()))
    }

    #[test]
    fn picks_the_first_existing_directory_skipping_missing_ones() {
        let base = temp_base("pick_first");
        let missing = base.join("missing");
        let present = base.join("present");
        std::fs::create_dir_all(&present).unwrap();

        let chosen = pick_existing_dir(&[missing, present.clone()]);

        assert_eq!(chosen, Some(present.clone()));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn returns_none_when_no_candidate_directory_exists() {
        let base = temp_base("pick_none");
        let a = base.join("a");
        let b = base.join("b");

        assert_eq!(pick_existing_dir(&[a, b]), None);
    }
}
