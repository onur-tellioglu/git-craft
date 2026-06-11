---
title: dabcraft M1 — Foundation (window, GPU, fly camera, packed-quad test section)
date: 2026-06-11
domain: infra
type: enhancement
priority: high
breaking: false
db-migration: false
rls-affecting: false
slice: 1
parent-spec: docs/superpowers/specs/2026-06-11-dabcraft-design.md
touched-files: [dabcraft/Cargo.toml, dabcraft/src/**, dabcraft/assets/shaders/*.wgsl]
trigger-tasks-touched: []
shared-modules-touched: []
---

# dabcraft M1 — Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native macOS window rendering one hardcoded 32³ voxel section through a vertex-pulling pipeline (packed 8-byte quads, no vertex buffer), explorable with a fly camera, with WGSL hot-reload and an egui FPS/GPU-time overlay.

**Architecture:** winit 0.30 `ApplicationHandler` drives the loop; wgpu 29 on Metal renders forward with a depth buffer. Quads are packed into 2×u32 on CPU, unpacked with `extractBits()` in the vertex shader. A naive per-face mesher (temporary; binary greedy meshing replaces it in M2) feeds the pipeline. Engine math and packing are pure functions, built TDD.

**Tech Stack:** Rust (edition 2024), wgpu 29, winit 0.30.13, glam 0.33, bytemuck, pollster, naga 29 (shader validation for hot-reload), egui/egui-wgpu/egui-winit 0.34.

**No GitHub issue:** repository has no remote yet; issue gate skipped. Plain commit messages, no `refs #N`.

**API warning for the implementer:** wgpu 29 broke many tutorial-era APIs. The code in this plan was verified against docs.rs for wgpu 29.0.1 / winit 0.30.13 / egui 0.34.0. If a snippet fails to compile, check docs.rs for the exact version in `Cargo.lock` before "fixing" it back to an older pattern. Known traps:
- `Instance::new` takes an `InstanceDescriptor` built via constructors (`new_with_display_handle_from_env(...)`), by value.
- `Surface::get_current_texture()` returns the `CurrentSurfaceTexture` enum, not `Result`.
- `PipelineLayoutDescriptor`: `bind_group_layouts: &[Option<&BindGroupLayout>]`, `push_constant_ranges` is gone (`immediate_size: u32`).
- `DepthStencilState::depth_write_enabled`/`depth_compare` are `Option`.
- `RenderPassColorAttachment` has a `depth_slice: None` field; `RenderPipelineDescriptor`/`RenderPassDescriptor` use `multiview_mask: None`.
- `request_adapter`/`request_device` return `Result` futures; `DeviceDescriptor` has `experimental_features` and `trace` fields.
- Apple GPUs do NOT support timestamps inside passes — only `RenderPassTimestampWrites` at pass boundaries.

---

## File Structure

All paths relative to repo root. The crate lives in `dabcraft/`.

```
dabcraft/
├── Cargo.toml
├── assets/
│   └── shaders/
│       └── terrain.wgsl          # vertex-pulling terrain shader
└── src/
    ├── main.rs                   # event loop bootstrap, wgpu Instance creation
    ├── app.rs                    # App: ApplicationHandler impl, owns all state, frame orchestration
    ├── game/
    │   ├── mod.rs
    │   ├── camera.rs             # Camera: position/yaw/pitch → view-projection (pure, tested)
    │   └── input.rs              # InputState: pressed keys + mouse deltas (pure, tested)
    ├── mesh/
    │   ├── mod.rs
    │   ├── quad.rs               # PackedQuad: 2×u32 pack/unpack (pure, tested)
    │   └── naive.rs              # temporary per-face mesher for M1 (pure, tested)
    ├── world/
    │   ├── mod.rs
    │   ├── block.rs              # BlockId newtype + AIR/test blocks
    │   └── section.rs            # Section: 32³ block array (pure, tested)
    └── render/
        ├── mod.rs
        ├── gpu.rs                # Gpu: surface/device/queue/config, resize, frame acquire
        ├── depth.rs              # depth texture creation/recreation
        ├── terrain.rs            # terrain pipeline, camera bind group, quad storage buffer, index buffer
        ├── hot_reload.rs         # WGSL mtime watcher + naga validation + pipeline swap
        ├── timestamps.rs         # QuerySet wrapper: per-pass GPU ms readback
        └── egui_layer.rs         # egui context/state/renderer wiring, debug window
```

Responsibilities: `app.rs` is the only module that knows about all the others; `game/`, `mesh/`, `world/` are GPU-free pure logic (unit-testable); `render/` owns everything wgpu.

---

### Task 1: Crate scaffold

**Files:**
- Create: `dabcraft/Cargo.toml`
- Create: `dabcraft/src/main.rs` (placeholder)
- Create: `dabcraft/assets/shaders/` (directory)

- [ ] **Step 1: Scaffold the crate**

```bash
cd /Users/onurtellioglu/Github/Minecraft
cargo new dabcraft
mkdir -p dabcraft/assets/shaders
```

- [ ] **Step 2: Write Cargo.toml**

Replace `dabcraft/Cargo.toml` with:

```toml
[package]
name = "dabcraft"
version = "0.1.0"
edition = "2024"

[dependencies]
wgpu = "29"
winit = "0.30.13"
glam = "0.33"
bytemuck = { version = "1.25", features = ["derive"] }
pollster = "0.4"
naga = { version = "29", features = ["wgsl-in"] }
egui = "0.34"
egui-wgpu = "0.34"
egui-winit = "0.34"
log = "0.4"
env_logger = "0.11"

[profile.dev]
opt-level = 1            # debug builds are unusably slow for voxel work at opt-level 0

[profile.dev.package."*"]
opt-level = 3            # always optimize dependencies (wgpu especially)
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build --manifest-path dabcraft/Cargo.toml`
Expected: compiles (downloads ~400 crates first time, several minutes). If `edition = "2024"` errors, your toolchain is too old — run `rustup update stable` (wgpu 29 needs Rust ≥ 1.87).

- [ ] **Step 4: Commit**

```bash
git add dabcraft/Cargo.toml dabcraft/src/main.rs
git commit -m "feat: scaffold dabcraft crate with wgpu 29 stack"
```

### Task 2: Window with ApplicationHandler

**Files:**
- Modify: `dabcraft/src/main.rs`
- Create: `dabcraft/src/app.rs`

No unit test possible (OS windowing); verification is build + manual run.

- [ ] **Step 1: Write main.rs**

```rust
mod app;

use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);

    // wgpu 29 requires the display handle at Instance creation time.
    let display_handle = event_loop.owned_display_handle();
    let instance = wgpu::Instance::new(
        wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display_handle)),
    );

    let mut app = app::App::new(instance);
    event_loop.run_app(&mut app).unwrap();
}
```

- [ ] **Step 2: Write app.rs**

```rust
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

pub struct App {
    instance: wgpu::Instance,
    window: Option<Arc<Window>>,
}

impl App {
    pub fn new(instance: wgpu::Instance) -> Self {
        Self { instance, window: None }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // macOS can resume more than once; init exactly once
        }
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("dabcraft"))
                .unwrap(),
        );
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape) && event.state.is_pressed() {
                    event_loop.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                // rendering lands here in Task 3
            }
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
```

- [ ] **Step 3: Run it**

Run: `cargo run --manifest-path dabcraft/Cargo.toml`
Expected: an empty window titled "dabcraft" opens; Escape or the close button exits cleanly.

- [ ] **Step 4: Commit**

```bash
git add dabcraft/src
git commit -m "feat: open window via winit ApplicationHandler"
```

### Task 3: GPU context and cleared frame

**Files:**
- Create: `dabcraft/src/render/mod.rs`
- Create: `dabcraft/src/render/gpu.rs`
- Create: `dabcraft/src/render/depth.rs`
- Modify: `dabcraft/src/main.rs` (add `mod render;`)
- Modify: `dabcraft/src/app.rs`

- [ ] **Step 1: Write render/mod.rs**

```rust
pub mod depth;
pub mod gpu;
```

(Add `mod render;` under `mod app;` in `main.rs`.)

- [ ] **Step 2: Write render/gpu.rs**

```rust
use std::sync::Arc;
use winit::window::Window;

pub struct Gpu {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
}

impl Gpu {
    pub fn new(instance: &wgpu::Instance, window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let surface = instance.create_surface(window).unwrap();

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .expect("no suitable GPU adapter");

        // TIMESTAMP_QUERY is in the spec from day one; fall back gracefully if absent.
        let mut required_features = wgpu::Features::empty();
        if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            required_features |= wgpu::Features::TIMESTAMP_QUERY;
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features,
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .expect("device request failed");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo, // Mailbox panics on Metal
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        Self { surface, device, queue, config }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Acquire the swapchain frame, handling every CurrentSurfaceTexture variant.
    /// Returns None when this frame should be skipped.
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                None
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => None,
            wgpu::CurrentSurfaceTexture::Validation => panic!("surface validation error"),
        }
    }
}
```

- [ ] **Step 3: Write render/depth.rs**

```rust
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
```

- [ ] **Step 4: Wire into app.rs**

Add fields and rendering. Updated `app.rs` structure (new/changed parts shown in full):

```rust
use crate::render::{depth, gpu::Gpu};

pub struct App {
    instance: wgpu::Instance,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    depth_view: Option<wgpu::TextureView>,
}

impl App {
    pub fn new(instance: wgpu::Instance) -> Self {
        Self { instance, window: None, gpu: None, depth_view: None }
    }

    fn render(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(frame) = gpu.acquire() else { return };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
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
                    view: self.depth_view.as_ref().unwrap(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard, // TBDR: not sampled later in M1
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
```

In `resumed()`, after creating the window:

```rust
        let gpu = Gpu::new(&self.instance, window.clone());
        let size = window.inner_size();
        self.depth_view = Some(depth::create_depth_view(&gpu.device, size.width, size.height));
        self.gpu = Some(gpu);
        self.window = Some(window);
```

In `window_event`, handle resize and redraw:

```rust
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                    self.depth_view = Some(depth::create_depth_view(&gpu.device, size.width, size.height));
                }
            }
            WindowEvent::RedrawRequested => self.render(),
```

- [ ] **Step 5: Run it**

Run: `cargo run --manifest-path dabcraft/Cargo.toml`
Expected: sky-blue window; resizing keeps it sky-blue without validation errors in the log.

- [ ] **Step 6: Commit**

```bash
git add dabcraft/src
git commit -m "feat: initialize wgpu surface, device and depth buffer; clear frame"
```

### Task 4: Camera math (TDD)

**Files:**
- Create: `dabcraft/src/game/mod.rs`
- Create: `dabcraft/src/game/camera.rs`
- Modify: `dabcraft/src/main.rs` (add `mod game;`)

Conventions (used by every later milestone — do not deviate): right-handed, +Y up. `yaw = 0, pitch = 0` looks toward **−Z**; positive yaw turns right (toward +X); pitch is clamped to ±89°. Projection is `Mat4::perspective_rh` (0..1 depth — wgpu's clip space; NOT the `_gl` variant).

- [ ] **Step 1: Write the failing tests**

`dabcraft/src/game/mod.rs`:

```rust
pub mod camera;
```

`dabcraft/src/game/camera.rs` (tests first; types referenced don't exist yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Vec4};

    fn approx(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-5, "{a} != {b}");
    }

    #[test]
    fn default_orientation_looks_down_negative_z() {
        let cam = Camera::new(Vec3::ZERO);
        approx(cam.forward(), Vec3::NEG_Z);
    }

    #[test]
    fn positive_yaw_turns_right() {
        let mut cam = Camera::new(Vec3::ZERO);
        cam.yaw = std::f32::consts::FRAC_PI_2; // 90° right
        approx(cam.forward(), Vec3::X);
    }

    #[test]
    fn pitch_is_clamped() {
        let mut cam = Camera::new(Vec3::ZERO);
        cam.apply_mouse_delta(0.0, -10_000.0); // huge upward look
        assert!(cam.pitch <= Camera::PITCH_LIMIT);
        cam.apply_mouse_delta(0.0, 10_000.0);
        assert!(cam.pitch >= -Camera::PITCH_LIMIT);
    }

    #[test]
    fn view_proj_maps_point_in_front_to_clip_space() {
        let cam = Camera::new(Vec3::ZERO);
        let vp = cam.view_proj(16.0 / 9.0);
        let clip = vp * Vec4::new(0.0, 0.0, -10.0, 1.0); // 10 units ahead
        let ndc = clip / clip.w;
        assert!(ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5, "centered point stays centered");
        assert!(ndc.z > 0.0 && ndc.z < 1.0, "wgpu depth range is 0..1, got {}", ndc.z);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml camera`
Expected: FAIL — `Camera` not found.

- [ ] **Step 3: Implement Camera**

Above the tests in `camera.rs`:

```rust
use glam::{Mat4, Vec3};

pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,   // radians, 0 = -Z, positive = right
    pub pitch: f32, // radians, clamped
    pub fov_y: f32, // radians
}

impl Camera {
    pub const PITCH_LIMIT: f32 = 89.0 * std::f32::consts::PI / 180.0;
    const MOUSE_SENSITIVITY: f32 = 0.0022;

    pub fn new(position: Vec3) -> Self {
        Self { position, yaw: 0.0, pitch: 0.0, fov_y: 70f32.to_radians() }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }

    /// winit MouseMotion delta: +x = mouse right, +y = mouse down.
    pub fn apply_mouse_delta(&mut self, dx: f64, dy: f64) {
        self.yaw += dx as f32 * Self::MOUSE_SENSITIVITY;
        self.pitch = (self.pitch - dy as f32 * Self::MOUSE_SENSITIVITY)
            .clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let proj = Mat4::perspective_rh(self.fov_y, aspect, 0.1, 1000.0);
        let view = Mat4::look_to_rh(self.position, self.forward(), Vec3::Y);
        proj * view
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml camera`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add dabcraft/src
git commit -m "feat: add fly camera math with clamped pitch and wgpu-space projection"
```

### Task 5: Input state and fly movement (TDD)

**Files:**
- Create: `dabcraft/src/game/input.rs`
- Modify: `dabcraft/src/game/mod.rs`, `dabcraft/src/game/camera.rs`, `dabcraft/src/app.rs`

Flight model (spec §7, M1 subset): WASD moves on the horizontal plane relative to yaw (forward = look direction projected to XZ), Space up, LeftShift down. Speed 20 blocks/s.

- [ ] **Step 1: Write the failing tests**

Add `pub mod input;` to `game/mod.rs`. `dabcraft/src/game/input.rs`:

```rust
use std::collections::HashSet;
use winit::keyboard::KeyCode;

#[derive(Default)]
pub struct InputState {
    pressed: HashSet<KeyCode>,
    pub mouse_delta: (f64, f64),
}

impl InputState {
    pub fn set_key(&mut self, key: KeyCode, down: bool) {
        if down {
            self.pressed.insert(key);
        } else {
            self.pressed.remove(&key);
        }
    }

    pub fn is_down(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    /// Mouse deltas accumulate across device events; reset once consumed each frame.
    pub fn take_mouse_delta(&mut self) -> (f64, f64) {
        std::mem::take(&mut self.mouse_delta)
    }
}
```

Add to `camera.rs` tests:

```rust
    #[test]
    fn fly_moves_horizontally_along_yaw_even_when_pitched() {
        let mut cam = Camera::new(Vec3::ZERO);
        cam.pitch = 1.0; // looking up
        let mut input = crate::game::input::InputState::default();
        input.set_key(winit::keyboard::KeyCode::KeyW, true);
        cam.fly(&input, 1.0);
        approx(cam.position, Vec3::new(0.0, 0.0, -Camera::FLY_SPEED)); // no vertical drift
    }

    #[test]
    fn space_and_shift_move_vertically() {
        let mut cam = Camera::new(Vec3::ZERO);
        let mut input = crate::game::input::InputState::default();
        input.set_key(winit::keyboard::KeyCode::Space, true);
        cam.fly(&input, 0.5);
        approx(cam.position, Vec3::new(0.0, Camera::FLY_SPEED * 0.5, 0.0));
    }

    #[test]
    fn opposing_keys_cancel() {
        let mut cam = Camera::new(Vec3::ZERO);
        let mut input = crate::game::input::InputState::default();
        input.set_key(winit::keyboard::KeyCode::KeyW, true);
        input.set_key(winit::keyboard::KeyCode::KeyS, true);
        cam.fly(&input, 1.0);
        approx(cam.position, Vec3::ZERO);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml camera`
Expected: FAIL — no method `fly`, no const `FLY_SPEED`.

- [ ] **Step 3: Implement fly()**

Add to `impl Camera`:

```rust
    pub const FLY_SPEED: f32 = 20.0; // blocks per second

    pub fn fly(&mut self, input: &crate::game::input::InputState, dt: f32) {
        use winit::keyboard::KeyCode as K;
        let forward = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin());
        let mut dir = Vec3::ZERO;
        if input.is_down(K::KeyW) { dir += forward; }
        if input.is_down(K::KeyS) { dir -= forward; }
        if input.is_down(K::KeyD) { dir += right; }
        if input.is_down(K::KeyA) { dir -= right; }
        if input.is_down(K::Space) { dir += Vec3::Y; }
        if input.is_down(K::ShiftLeft) { dir -= Vec3::Y; }
        if dir != Vec3::ZERO {
            self.position += dir.normalize() * Self::FLY_SPEED * dt;
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml camera`
Expected: 7 passed.

- [ ] **Step 5: Wire into app.rs**

Add fields to `App`: `input: InputState`, `camera: Camera`, `last_frame: std::time::Instant`. Initialize in `new()` (`Camera::new(glam::Vec3::new(16.0, 40.0, 60.0))` — looks at the test section from outside).

In `resumed()`, lock the cursor after window creation:

```rust
        let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Locked);
        window.set_cursor_visible(false);
```

In `window_event` keyboard handling, forward every key to input (keep Escape exit):

```rust
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if code == KeyCode::Escape && event.state.is_pressed() {
                        event_loop.exit();
                    }
                    self.input.set_key(code, event.state.is_pressed());
                }
            }
```

In `device_event`:

```rust
        if let DeviceEvent::MouseMotion { delta } = event {
            self.input.mouse_delta.0 += delta.0;
            self.input.mouse_delta.1 += delta.1;
        }
```

At the top of `render()` (before acquiring the frame):

```rust
        let now = std::time::Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let (dx, dy) = self.input.take_mouse_delta();
        self.camera.apply_mouse_delta(dx, dy);
        self.camera.fly(&self.input, dt);
```

- [ ] **Step 6: Build and run**

Run: `cargo run --manifest-path dabcraft/Cargo.toml`
Expected: window opens, cursor disappears (camera has nothing to show yet — that's Task 8). Escape exits.

- [ ] **Step 7: Commit**

```bash
git add dabcraft/src
git commit -m "feat: add input state and fly camera movement"
```

### Task 6: Packed quad format (TDD)

**Files:**
- Create: `dabcraft/src/mesh/mod.rs`
- Create: `dabcraft/src/mesh/quad.rs`
- Modify: `dabcraft/src/main.rs` (add `mod mesh;`)

The 8-byte quad is the contract between CPU meshing and the WGSL vertex shader (spec §5). Bit layout — the WGSL unpack in Task 7 must mirror this exactly:

```
data0: bits 0-5 x | 6-11 y | 12-17 z | 18-20 face | 21-25 (w-1)   [6 spare]
data1: bits 0-4 (h-1) | 5-12 ao (4×2, corner order 00,10,11,01) | 13-16 skylight | 17-20 blocklight | 21-30 texture layer
```

`x,y,z` are 0..33 (room for the 34³ apron coordinates from M2 on); `face`: 0=+X, 1=−X, 2=+Y, 3=−Y, 4=+Z, 5=−Z; `w,h` are 1..32 stored minus one.

- [ ] **Step 1: Write the failing tests**

`dabcraft/src/mesh/mod.rs`:

```rust
pub mod quad;
```

`dabcraft/src/mesh/quad.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(q: Quad) {
        assert_eq!(PackedQuad::pack(q).unpack(), q);
    }

    #[test]
    fn packs_and_unpacks_all_fields() {
        roundtrip(Quad {
            x: 12, y: 33, z: 7, face: 4, w: 32, h: 1,
            ao: [0, 1, 2, 3], skylight: 15, blocklight: 9, texture: 1000,
        });
    }

    #[test]
    fn packs_field_extremes() {
        roundtrip(Quad { x: 0, y: 0, z: 0, face: 0, w: 1, h: 1, ao: [0; 4], skylight: 0, blocklight: 0, texture: 0 });
        roundtrip(Quad { x: 33, y: 33, z: 33, face: 5, w: 32, h: 32, ao: [3; 4], skylight: 15, blocklight: 15, texture: 1023 });
    }

    #[test]
    fn packed_quad_is_8_bytes() {
        assert_eq!(std::mem::size_of::<PackedQuad>(), 8);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml quad`
Expected: FAIL — `Quad`/`PackedQuad` not found.

- [ ] **Step 3: Implement**

Above the tests in `quad.rs`:

```rust
use bytemuck::{Pod, Zeroable};

/// Unpacked quad, CPU-side working representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quad {
    pub x: u32,        // 0..=33 (apron space)
    pub y: u32,
    pub z: u32,
    pub face: u32,     // 0..=5: +X -X +Y -Y +Z -Z
    pub w: u32,        // 1..=32, extent along the face's U axis
    pub h: u32,        // 1..=32, extent along the face's V axis
    pub ao: [u32; 4],  // 0..=3 per corner, order: (0,0) (w,0) (w,h) (0,h)
    pub skylight: u32, // 0..=15
    pub blocklight: u32,
    pub texture: u32,  // 0..=1023, texture array layer
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PackedQuad {
    pub data0: u32,
    pub data1: u32,
}

impl PackedQuad {
    pub fn pack(q: Quad) -> Self {
        debug_assert!(q.x < 34 && q.y < 34 && q.z < 34 && q.face < 6);
        debug_assert!((1..=32).contains(&q.w) && (1..=32).contains(&q.h));
        debug_assert!(q.skylight < 16 && q.blocklight < 16 && q.texture < 1024);
        let data0 = q.x | (q.y << 6) | (q.z << 12) | (q.face << 18) | ((q.w - 1) << 21);
        let ao = q.ao[0] | (q.ao[1] << 2) | (q.ao[2] << 4) | (q.ao[3] << 6);
        let data1 = (q.h - 1) | (ao << 5) | (q.skylight << 13) | (q.blocklight << 17) | (q.texture << 21);
        Self { data0, data1 }
    }

    pub fn unpack(self) -> Quad {
        let bits = |v: u32, off: u32, n: u32| (v >> off) & ((1 << n) - 1);
        let ao_bits = bits(self.data1, 5, 8);
        Quad {
            x: bits(self.data0, 0, 6),
            y: bits(self.data0, 6, 6),
            z: bits(self.data0, 12, 6),
            face: bits(self.data0, 18, 3),
            w: bits(self.data0, 21, 5) + 1,
            h: bits(self.data1, 0, 5) + 1,
            ao: [ao_bits & 3, (ao_bits >> 2) & 3, (ao_bits >> 4) & 3, (ao_bits >> 6) & 3],
            skylight: bits(self.data1, 13, 4),
            blocklight: bits(self.data1, 17, 4),
            texture: bits(self.data1, 21, 10),
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml quad`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add dabcraft/src
git commit -m "feat: add 8-byte packed quad format with roundtrip tests"
```

### Task 7: Terrain shader and vertex-pulling pipeline

**Files:**
- Create: `dabcraft/assets/shaders/terrain.wgsl`
- Create: `dabcraft/src/render/terrain.rs`
- Modify: `dabcraft/src/render/mod.rs` (add `pub mod terrain;`)
- Modify: `dabcraft/src/mesh/mod.rs`

- [ ] **Step 1: Write the index helper test (TDD for the pure part)**

Add to `dabcraft/src/mesh/quad.rs` tests:

```rust
    #[test]
    fn quad_indices_reference_four_vertices_per_quad() {
        assert_eq!(build_quad_indices(2), vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
    }
```

Run: `cargo test --manifest-path dabcraft/Cargo.toml quad` → FAIL (`build_quad_indices` not found). Implement in `quad.rs`:

```rust
/// Two CCW triangles per quad: (0,1,2) and (0,2,3), vertices 4i..4i+3.
pub fn build_quad_indices(quad_count: u32) -> Vec<u32> {
    let mut indices = Vec::with_capacity(quad_count as usize * 6);
    for i in 0..quad_count {
        let b = i * 4;
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    indices
}
```

Run again → PASS.

- [ ] **Step 2: Write terrain.wgsl**

The unpack constants here MUST mirror `PackedQuad::pack` bit-for-bit. M1 has no textures yet: the `texture` field carries a block id, colored from a palette; real texture arrays arrive in M6.

```wgsl
struct CameraUniform {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;

// Per-face: origin offset (added to voxel pos), U axis, V axis.
// Face order matches Rust: 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z.
const FACE_ORIGIN = array<vec3<f32>, 6>(
    vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 0.0),
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 0.0),
);
const FACE_U = array<vec3<f32>, 6>(
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0),
    vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0),
    vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0),
);
const FACE_V = array<vec3<f32>, 6>(
    vec3(0.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0),
    vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0),
    vec3(0.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0),
);
// Minecraft-style face shading: top, bottom, ±X, ±Z.
const FACE_SHADE = array<f32, 6>(0.8, 0.8, 1.0, 0.5, 0.6, 0.6);

// M1 block palette, indexed by the quad's texture field.
const PALETTE = array<vec3<f32>, 4>(
    vec3(1.0, 0.0, 1.0),      // 0 = air (never rendered; magenta = bug)
    vec3(0.35, 0.62, 0.22),   // 1 = grass
    vec3(0.45, 0.32, 0.2),    // 2 = dirt
    vec3(0.52, 0.52, 0.54),   // 3 = stone
);

// Corner order matches PackedQuad ao order: (0,0) (w,0) (w,h) (0,h).
const CORNER_UV = array<vec2<f32>, 4>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let quad = quads[vi / 4u];
    let corner = vi % 4u;

    let x = f32(extractBits(quad.x, 0u, 6u));
    let y = f32(extractBits(quad.x, 6u, 6u));
    let z = f32(extractBits(quad.x, 12u, 6u));
    let face = extractBits(quad.x, 18u, 3u);
    let w = f32(extractBits(quad.x, 21u, 5u) + 1u);
    let h = f32(extractBits(quad.y, 0u, 5u) + 1u);
    let ao = f32(extractBits(quad.y, 5u + corner * 2u, 2u));
    let skylight = f32(extractBits(quad.y, 13u, 4u));
    let tex = extractBits(quad.y, 21u, 10u);

    let uv = CORNER_UV[corner];
    let pos = vec3(x, y, z) + FACE_ORIGIN[face] + FACE_U[face] * uv.x * w + FACE_V[face] * uv.y * h;

    var out: VsOut;
    out.clip = camera.view_proj * vec4(pos, 1.0);
    let light = (skylight / 15.0) * FACE_SHADE[face] * mix(0.4, 1.0, ao / 3.0);
    out.color = PALETTE[min(tex, 3u)] * light;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
```

- [ ] **Step 3: Write render/terrain.rs**

```rust
use wgpu::util::DeviceExt;

use crate::mesh::quad::{build_quad_indices, PackedQuad};
use crate::render::depth::DEPTH_FORMAT;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

pub struct TerrainRenderer {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_layout: wgpu::BindGroupLayout,
    quads_layout: wgpu::BindGroupLayout,
    quads_bind_group: Option<wgpu::BindGroup>,
    index_buffer: Option<wgpu::Buffer>,
    index_count: u32,
    surface_format: wgpu::TextureFormat,
}

impl TerrainRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat, shader_source: &str) -> Self {
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let quads_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quads"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });

        let pipeline = Self::build_pipeline(device, surface_format, &camera_layout, &quads_layout, shader_source);

        Self {
            pipeline,
            camera_buffer,
            camera_bind_group,
            camera_layout,
            quads_layout,
            quads_bind_group: None,
            index_buffer: None,
            index_count: 0,
            surface_format,
        }
    }

    fn build_pipeline(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        quads_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(camera_layout), Some(quads_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[], // vertex pulling: no vertex buffers
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None, // M2 establishes winding + backface culling
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Replace the pipeline with one built from new shader source (hot-reload, Task 9).
    pub fn swap_shader(&mut self, device: &wgpu::Device, shader_source: &str) {
        self.pipeline = Self::build_pipeline(
            device, self.surface_format, &self.camera_layout, &self.quads_layout, shader_source,
        );
    }

    pub fn upload_quads(&mut self, device: &wgpu::Device, quads: &[PackedQuad]) {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quads"),
            contents: bytemuck::cast_slice(quads),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.quads_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quads"),
            layout: &self.quads_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        }));
        let indices = build_quad_indices(quads.len() as u32);
        self.index_buffer = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        }));
        self.index_count = indices.len() as u32;
    }

    pub fn write_camera(&self, queue: &wgpu::Queue, view_proj: glam::Mat4) {
        let uniform = CameraUniform { view_proj: view_proj.to_cols_array_2d() };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass<'_>) {
        let (Some(quads_bg), Some(index_buffer)) = (&self.quads_bind_group, &self.index_buffer) else {
            return;
        };
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.camera_bind_group, &[]);
        rpass.set_bind_group(1, quads_bg, &[]);
        rpass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
```

- [ ] **Step 4: Build**

Run: `cargo build --manifest-path dabcraft/Cargo.toml && cargo test --manifest-path dabcraft/Cargo.toml`
Expected: compiles; all tests pass. (Visual verification comes in Task 8 when quads exist.)

- [ ] **Step 5: Commit**

```bash
git add dabcraft/src dabcraft/assets
git commit -m "feat: add vertex-pulling terrain pipeline and WGSL quad unpack shader"
```

### Task 8: Test section and naive mesher (TDD) — first visible geometry

**Files:**
- Create: `dabcraft/src/world/mod.rs`, `dabcraft/src/world/block.rs`, `dabcraft/src/world/section.rs`
- Create: `dabcraft/src/mesh/naive.rs`
- Modify: `dabcraft/src/main.rs` (add `mod world;`), `dabcraft/src/mesh/mod.rs`, `dabcraft/src/app.rs`

The naive mesher (one quad per exposed voxel face, no merging) is deliberately temporary — M2 replaces it with binary greedy meshing. Its tests define behavior the greedy mesher must also satisfy.

- [ ] **Step 1: Write block.rs and section.rs with failing tests**

`world/mod.rs`:

```rust
pub mod block;
pub mod section;
```

`world/block.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockId(pub u16);

pub const AIR: BlockId = BlockId(0);
pub const GRASS: BlockId = BlockId(1);
pub const DIRT: BlockId = BlockId(2);
pub const STONE: BlockId = BlockId(3);

impl BlockId {
    pub fn is_solid(self) -> bool {
        self != AIR
    }
}
```

`world/section.rs`:

```rust
use crate::world::block::{BlockId, AIR};

pub const SECTION_SIZE: usize = 32;

pub struct Section {
    blocks: Box<[BlockId; SECTION_SIZE * SECTION_SIZE * SECTION_SIZE]>,
}

impl Section {
    pub fn empty() -> Self {
        Self { blocks: Box::new([AIR; SECTION_SIZE * SECTION_SIZE * SECTION_SIZE]) }
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < SECTION_SIZE && y < SECTION_SIZE && z < SECTION_SIZE);
        (y * SECTION_SIZE + z) * SECTION_SIZE + x
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockId {
        self.blocks[Self::index(x, y, z)]
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, block: BlockId) {
        self.blocks[Self::index(x, y, z)] = block;
    }

    /// Out-of-bounds counts as air (M1: no neighbor apron yet).
    pub fn get_or_air(&self, x: i32, y: i32, z: i32) -> BlockId {
        let r = 0..SECTION_SIZE as i32;
        if r.contains(&x) && r.contains(&y) && r.contains(&z) {
            self.get(x as usize, y as usize, z as usize)
        } else {
            AIR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::{AIR, STONE};

    #[test]
    fn set_then_get_roundtrips() {
        let mut s = Section::empty();
        s.set(31, 0, 17, STONE);
        assert_eq!(s.get(31, 0, 17), STONE);
        assert_eq!(s.get(0, 0, 0), AIR);
    }

    #[test]
    fn out_of_bounds_is_air() {
        let mut s = Section::empty();
        s.set(0, 0, 0, STONE);
        assert_eq!(s.get_or_air(-1, 0, 0), AIR);
        assert_eq!(s.get_or_air(0, 32, 0), AIR);
        assert_eq!(s.get_or_air(0, 0, 0), STONE);
    }
}
```

Run: `cargo test --manifest-path dabcraft/Cargo.toml section` → 2 passed (implementation written together with tests here; the genuinely test-first part is the mesher below).

- [ ] **Step 2: Write failing mesher tests**

Add `pub mod naive;` to `mesh/mod.rs`. `dabcraft/src/mesh/naive.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::STONE;
    use crate::world::section::Section;

    #[test]
    fn empty_section_yields_no_quads() {
        assert!(mesh_naive(&Section::empty()).is_empty());
    }

    #[test]
    fn single_block_yields_six_quads() {
        let mut s = Section::empty();
        s.set(5, 5, 5, STONE);
        assert_eq!(mesh_naive(&s).len(), 6);
    }

    #[test]
    fn touching_faces_are_culled() {
        let mut s = Section::empty();
        s.set(5, 5, 5, STONE);
        s.set(6, 5, 5, STONE);
        // 12 faces total, 2 shared (hidden) => 10
        assert_eq!(mesh_naive(&s).len(), 10);
    }

    #[test]
    fn full_floor_slab_face_count() {
        let mut s = Section::empty();
        for x in 0..32 {
            for z in 0..32 {
                s.set(x, 0, z, STONE);
            }
        }
        // top 1024 + bottom 1024 + 4 sides * 32 = 2176
        assert_eq!(mesh_naive(&s).len(), 2176);
    }

    #[test]
    fn quads_carry_block_id_as_texture() {
        let mut s = Section::empty();
        s.set(0, 0, 0, STONE);
        let quads = mesh_naive(&s);
        assert!(quads.iter().all(|q| q.unpack().texture == STONE.0 as u32));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml naive`
Expected: FAIL — `mesh_naive` not found.

- [ ] **Step 4: Implement mesh_naive**

Above the tests in `naive.rs`:

```rust
use crate::mesh::quad::{PackedQuad, Quad};
use crate::world::section::{Section, SECTION_SIZE};

/// Face order matches the packed format: +X -X +Y -Y +Z -Z.
const NEIGHBOR_OFFSETS: [(i32, i32, i32); 6] =
    [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];

/// One 1x1 quad per exposed face. Temporary M1 mesher; M2 replaces it
/// with binary greedy meshing behind the same output contract.
pub fn mesh_naive(section: &Section) -> Vec<PackedQuad> {
    let mut quads = Vec::new();
    for y in 0..SECTION_SIZE {
        for z in 0..SECTION_SIZE {
            for x in 0..SECTION_SIZE {
                let block = section.get(x, y, z);
                if !block.is_solid() {
                    continue;
                }
                for (face, (dx, dy, dz)) in NEIGHBOR_OFFSETS.iter().enumerate() {
                    let neighbor = section.get_or_air(x as i32 + dx, y as i32 + dy, z as i32 + dz);
                    if neighbor.is_solid() {
                        continue;
                    }
                    quads.push(PackedQuad::pack(Quad {
                        x: x as u32,
                        y: y as u32,
                        z: z as u32,
                        face: face as u32,
                        w: 1,
                        h: 1,
                        ao: [3; 4],        // real AO arrives in M2 with the apron
                        skylight: 15,      // real flood-fill light arrives in M4
                        blocklight: 0,
                        texture: block.0 as u32,
                    }));
                }
            }
        }
    }
    quads
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml naive`
Expected: 5 passed.

- [ ] **Step 6: Build the test scene and wire rendering in app.rs**

Add a helper in `app.rs`:

```rust
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
```

In `resumed()`, after creating `gpu` (shader loaded from disk — required for hot-reload in Task 9):

```rust
        let shader_path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/terrain.wgsl");
        let shader_source = std::fs::read_to_string(shader_path).expect("terrain.wgsl missing");
        let mut terrain = crate::render::terrain::TerrainRenderer::new(&gpu.device, gpu.config.format, &shader_source);
        terrain.upload_quads(&gpu.device, &crate::mesh::naive::mesh_naive(&build_test_section()));
        self.terrain = Some(terrain);
```

In `render()`, before the encoder is created:

```rust
        let aspect = gpu.config.width as f32 / gpu.config.height as f32;
        let terrain = self.terrain.as_ref().unwrap();
        terrain.write_camera(&gpu.queue, self.camera.view_proj(aspect));
```

Inside the render pass block (replace `let _rpass` with `let mut rpass`):

```rust
            terrain.draw(&mut rpass);
```

- [ ] **Step 7: Run and look**

Run: `cargo run --manifest-path dabcraft/Cargo.toml`
Expected: a grass-topped 32×32 island with a stone pillar and a floating dirt cube, fully explorable with WASD/Space/Shift + mouse, correct depth (pillar occludes terrain behind it). Top faces brightest, bottoms darkest.

- [ ] **Step 8: Commit**

```bash
git add dabcraft/src
git commit -m "feat: render naive-meshed test section through vertex pulling"
```

### Task 9: WGSL hot-reload

**Files:**
- Create: `dabcraft/src/render/hot_reload.rs`
- Modify: `dabcraft/src/render/mod.rs`, `dabcraft/src/app.rs`

Strategy: poll the file's mtime every 0.5 s; on change, validate with naga first (parse + validate), and only swap the pipeline if valid — a broken shader must never crash the app or kill the old pipeline (spec §3).

- [ ] **Step 1: Write the failing validation tests**

`dabcraft/src/render/hot_reload.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_wgsl_passes() {
        assert!(validate_wgsl("@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4(0.0); }").is_ok());
    }

    #[test]
    fn syntax_error_is_reported_not_panicked() {
        assert!(validate_wgsl("@vertex fn vs( {").is_err());
    }

    #[test]
    fn type_error_is_reported() {
        assert!(validate_wgsl("@vertex fn vs() -> @builtin(position) vec4<f32> { return 1; }").is_err());
    }

    #[test]
    fn shipped_terrain_shader_is_valid() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/terrain.wgsl")).unwrap();
        assert!(validate_wgsl(&src).is_ok());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path dabcraft/Cargo.toml hot_reload`
Expected: FAIL — `validate_wgsl` not found.

- [ ] **Step 3: Implement watcher + validation**

Above the tests:

```rust
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

pub fn validate_wgsl(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.to_string())?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub struct ShaderWatcher {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    last_check: Instant,
}

impl ShaderWatcher {
    const POLL_INTERVAL: Duration = Duration::from_millis(500);

    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let last_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Self { path, last_mtime, last_check: Instant::now() }
    }

    /// Returns validated new shader source when the file changed and is valid.
    pub fn poll(&mut self) -> Option<String> {
        if self.last_check.elapsed() < Self::POLL_INTERVAL {
            return None;
        }
        self.last_check = Instant::now();
        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok()?;
        if Some(mtime) == self.last_mtime {
            return None;
        }
        self.last_mtime = Some(mtime);
        let source = std::fs::read_to_string(&self.path).ok()?;
        match validate_wgsl(&source) {
            Ok(()) => {
                log::info!("shader reloaded: {}", self.path.display());
                Some(source)
            }
            Err(e) => {
                log::error!("shader error (keeping previous pipeline):\n{e}");
                None
            }
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path dabcraft/Cargo.toml hot_reload`
Expected: 4 passed.

- [ ] **Step 5: Wire into app.rs**

Add field `shader_watcher: Option<ShaderWatcher>`; create it in `resumed()` with the same `shader_path`. At the top of `render()`:

```rust
        if let (Some(watcher), Some(terrain), Some(gpu)) =
            (self.shader_watcher.as_mut(), self.terrain.as_mut(), self.gpu.as_ref())
        {
            if let Some(source) = watcher.poll() {
                terrain.swap_shader(&gpu.device, &source);
            }
        }
```

- [ ] **Step 6: Manual verification**

Run the app (`RUST_LOG=info cargo run --manifest-path dabcraft/Cargo.toml`). While it runs:
1. Edit `terrain.wgsl`: change the grass palette entry to `vec3(0.9, 0.2, 0.2)`. Save. Expected: terrain top turns red within ~1 s, no restart.
2. Introduce a syntax error (delete a `;`). Save. Expected: error logged, app keeps rendering with the previous shader.
3. Fix the error. Expected: recovers.
Revert the palette change afterwards.

- [ ] **Step 7: Commit**

```bash
git add dabcraft/src
git commit -m "feat: add WGSL hot-reload with naga validation"
```

### Task 10: GPU timestamps and egui debug HUD

**Files:**
- Create: `dabcraft/src/render/timestamps.rs`
- Create: `dabcraft/src/render/egui_layer.rs`
- Modify: `dabcraft/src/render/mod.rs`, `dabcraft/src/app.rs`

Apple GPUs only support timestamps at **pass boundaries** (`RenderPassTimestampWrites`); `write_timestamp` inside passes is unavailable on M-series. The readback is asynchronous: a frame's measurement appears a few frames later — that is fine for a HUD.

- [ ] **Step 1: Write render/timestamps.rs**

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Measures one render pass's GPU time via pass-boundary timestamps.
/// Readback is async: `last_ms` lags a few frames behind.
pub struct GpuTimer {
    query_set: Option<wgpu::QuerySet>,
    resolve_buffer: wgpu::Buffer,
    read_buffer: wgpu::Buffer,
    map_done: Arc<AtomicBool>,
    pending: bool,
    pub last_ms: f32,
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device) -> Self {
        let enabled = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let query_set = enabled.then(|| {
            device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            })
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts resolve"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts read"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            query_set,
            resolve_buffer,
            read_buffer,
            map_done: Arc::new(AtomicBool::new(false)),
            pending: false,
            last_ms: 0.0,
        }
    }

    /// Attach to a RenderPassDescriptor's `timestamp_writes`. None while a readback is in flight.
    pub fn pass_writes(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if self.pending {
            return None;
        }
        self.query_set.as_ref().map(|qs| wgpu::RenderPassTimestampWrites {
            query_set: qs,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        })
    }

    /// Call after the measured pass ended, before encoder.finish().
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.pending {
            return;
        }
        if let Some(qs) = &self.query_set {
            encoder.resolve_query_set(qs, 0..2, &self.resolve_buffer, 0);
            encoder.copy_buffer_to_buffer(&self.resolve_buffer, 0, &self.read_buffer, 0, 16);
        }
    }

    /// Call once per frame after queue.submit().
    pub fn after_submit(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.query_set.is_none() {
            return;
        }
        if !self.pending {
            let done = self.map_done.clone();
            // NOTE: if map_async's receiver form differs in your wgpu patch
            // version, check docs.rs — in 29 it is callable on Buffer with a range.
            self.read_buffer.map_async(wgpu::MapMode::Read, .., move |result| {
                if result.is_ok() {
                    done.store(true, Ordering::Release);
                }
            });
            self.pending = true;
            return;
        }
        // PollType::Poll = non-blocking pump. If the variant name differs on
        // your wgpu patch version, check wgpu::PollType docs.
        let _ = device.poll(wgpu::PollType::Poll);
        if self.map_done.swap(false, Ordering::AcqRel) {
            {
                let data = self.read_buffer.get_mapped_range(..);
                let ts: &[u64] = bytemuck::cast_slice(&data);
                let ns = ts[1].wrapping_sub(ts[0]) as f32 * queue.get_timestamp_period();
                self.last_ms = ns / 1_000_000.0;
            }
            self.read_buffer.unmap();
            self.pending = false;
        }
    }
}
```

- [ ] **Step 2: Write render/egui_layer.rs**

```rust
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
        let renderer = egui_wgpu::Renderer::new(device, surface_format, egui_wgpu::RendererOptions::default());
        Self { ctx, state, renderer }
    }

    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// Runs the UI closure, encodes egui into its own render pass (LoadOp::Load,
    /// drawn over the 3D frame). Returns command buffers that must be submitted
    /// BEFORE the main encoder's buffer.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        window: &Arc<Window>,
        view: &wgpu::TextureView,
        config: &wgpu::SurfaceConfiguration,
        ui: impl FnMut(&egui::Context),
    ) -> Vec<wgpu::CommandBuffer> {
        let raw_input = self.state.take_egui_input(window);
        let output = self.ctx.run(raw_input, ui);
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
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime(); // egui_wgpu 0.34 requires RenderPass<'static>
            self.renderer.render(&mut rpass, &paint_jobs, &screen);
        }
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        user_cmds
    }
}
```

- [ ] **Step 3: Wire into app.rs**

New `App` fields: `egui: Option<EguiLayer>`, `timer: Option<GpuTimer>`, `hud_visible: bool` (default `true`), `fps_smoothed: f32`, `quad_count: u32`.

In `resumed()`: `self.egui = Some(EguiLayer::new(&gpu.device, gpu.config.format, &window));` and `self.timer = Some(GpuTimer::new(&gpu.device));`. Store the quad count when uploading (`self.quad_count = quads.len() as u32`).

In `window_event`, feed egui FIRST (before the existing match) and toggle the HUD:

```rust
        if let (Some(egui), Some(window)) = (self.egui.as_mut(), self.window.as_ref()) {
            if egui.on_window_event(window, &event) {
                return;
            }
        }
        // in the KeyboardInput arm, alongside Escape:
        // if code == KeyCode::F3 && event.state.is_pressed() { self.hud_visible = !self.hud_visible; }
```

In `render()`:
- smooth FPS after computing `dt`: `self.fps_smoothed = self.fps_smoothed * 0.95 + (1.0 / dt.max(1e-6)) * 0.05;`
- main pass descriptor: `timestamp_writes: self.timer.as_ref().and_then(|t| t.pass_writes()),`
- after the main pass block: `self.timer.as_ref().unwrap().resolve(&mut encoder);`
- then egui (before submit):

```rust
        let mut egui_cmds = Vec::new();
        if self.hud_visible {
            let (fps, gpu_ms, quads) =
                (self.fps_smoothed, self.timer.as_ref().unwrap().last_ms, self.quad_count);
            egui_cmds = self.egui.as_mut().unwrap().draw(
                &gpu.device, &gpu.queue, &mut encoder,
                self.window.as_ref().unwrap(), &view, &gpu.config,
                |ctx| {
                    egui::Window::new("dabcraft debug")
                        .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
                        .resizable(false)
                        .show(ctx, |ui| {
                            ui.label(format!("fps: {fps:.0}  ({:.2} ms)", 1000.0 / fps.max(1.0)));
                            ui.label(format!("main pass gpu: {gpu_ms:.2} ms"));
                            ui.label(format!("quads: {quads}"));
                        });
                },
            );
        }
- replace the submit line:

        gpu.queue.submit(egui_cmds.into_iter().chain(Some(encoder.finish())));
        frame.present();
        self.timer.as_mut().unwrap().after_submit(&gpu.device, &gpu.queue);
```

- [ ] **Step 4: Run and verify**

Run: `cargo run --manifest-path dabcraft/Cargo.toml`
Expected: overlay top-left shows fps ≈ 120 (ProMotion) or 60, a plausible sub-millisecond main-pass GPU time, and quads = 2176 + landmark quads. F3 toggles the overlay. All tests still pass: `cargo test --manifest-path dabcraft/Cargo.toml`.

- [ ] **Step 5: Commit**

```bash
git add dabcraft/src
git commit -m "feat: add GPU pass timing and egui debug HUD"
```

---

## M1 Completion Checklist

- [ ] `cargo test --manifest-path dabcraft/Cargo.toml` — all green (≥ 21 tests)
- [ ] `cargo clippy --manifest-path dabcraft/Cargo.toml -- -D warnings` — clean (fix or explicitly allow with a reason)
- [ ] Manual: fly around the island at smooth fps, hot-reload works, HUD numbers sane
- [ ] Working tree clean, all commits pushed once a remote exists

**What M1 deliberately leaves wrong (fixed in M2+):** naive mesher (→ binary greedy), `cull_mode: None` (→ winding + backface culling), constant AO/light values (→ real AO at meshing, flood-fill in M4), quad buffer recreated on upload (→ arena allocator), single hardcoded section (→ chunk streaming).

## Self-Review Notes (already applied)

- Spec coverage: M1 scope from spec §10 fully covered — window/ApplicationHandler (T2), surface+depth (T3), fly camera (T4-5), packed quads (T6), vertex pulling (T7), test section (T8), hot-reload (T9); timestamp HUD pulled in from spec §8's "from day one" requirement (T10).
- Type consistency: `PackedQuad` bit layout identical between `quad.rs` (T6) and `terrain.wgsl` (T7) — verified field-by-field; `Camera::fly` signature matches tests; `TerrainRenderer::swap_shader` used by T9 is defined in T7.
- Known uncertainty flagged inline (not a placeholder): `map_async`/`PollType` exact forms in Task 10 carry doc-check notes because subagent verification reported them at lower confidence.

