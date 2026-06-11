use std::collections::HashSet;
use winit::keyboard::KeyCode;

#[derive(Default)]
pub struct InputState {
    pressed: HashSet<KeyCode>,
    mouse_delta: (f64, f64),
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

    pub fn accumulate_mouse(&mut self, dx: f64, dy: f64) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    /// Mouse deltas accumulate across device events; reset once consumed each frame.
    pub fn take_mouse_delta(&mut self) -> (f64, f64) {
        std::mem::take(&mut self.mouse_delta)
    }

    /// winit does not synthesize key-release events for keys held when focus
    /// is lost; stale state must be dropped on focus transitions.
    pub fn clear(&mut self) {
        self.pressed.clear();
        self.mouse_delta = (0.0, 0.0);
    }
}
