use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

pub struct App {
    #[allow(dead_code)]
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
