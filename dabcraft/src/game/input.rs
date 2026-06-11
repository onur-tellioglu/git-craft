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
