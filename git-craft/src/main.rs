mod app;
mod bench;
mod game;
mod mesh;
mod render;
mod world;

use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    env_logger::init();
    let bench_cfg = bench::parse_bench_args(std::env::args());

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);

    // wgpu 29 requires the display handle at Instance creation time.
    let display_handle = event_loop.owned_display_handle();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
        Box::new(display_handle),
    ));

    let mut app = app::App::new(instance, bench_cfg);
    event_loop.run_app(&mut app).unwrap();
}
