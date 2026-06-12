use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// Game-relevant mouse buttons (winit's enum carries more; app.rs maps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
}

#[derive(Default)]
pub struct InputState {
    pressed: HashSet<KeyCode>,
    just_pressed: HashSet<KeyCode>,
    mouse_held: HashSet<MouseButton>,
    mouse_just_pressed: HashSet<MouseButton>,
    mouse_delta: (f64, f64),
    scroll: f32,
}

impl InputState {
    pub fn set_key(&mut self, key: KeyCode, down: bool) {
        if down {
            if self.pressed.insert(key) {
                self.just_pressed.insert(key);
            }
        } else {
            self.pressed.remove(&key);
        }
    }

    pub fn is_down(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    /// True only on the frame the key transitioned up→down (OS key-repeat
    /// does not re-fire). Cleared by `end_frame`.
    pub fn key_pressed(&self, key: KeyCode) -> bool {
        self.just_pressed.contains(&key)
    }

    pub fn set_mouse_button(&mut self, button: MouseButton, down: bool) {
        if down {
            if self.mouse_held.insert(button) {
                self.mouse_just_pressed.insert(button);
            }
        } else {
            self.mouse_held.remove(&button);
        }
    }

    pub fn mouse_down(&self, button: MouseButton) -> bool {
        self.mouse_held.contains(&button)
    }

    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.mouse_just_pressed.contains(&button)
    }

    pub fn accumulate_mouse(&mut self, dx: f64, dy: f64) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    /// Mouse deltas accumulate across device events; reset once consumed each frame.
    pub fn take_mouse_delta(&mut self) -> (f64, f64) {
        std::mem::take(&mut self.mouse_delta)
    }

    pub fn accumulate_scroll(&mut self, delta: f32) {
        self.scroll += delta;
    }

    /// Whole scroll steps accumulated since the last call; the fractional
    /// remainder (trackpad deltas) carries over so slow scrolls still land.
    pub fn take_scroll_steps(&mut self) -> i32 {
        let steps = self.scroll.trunc() as i32;
        self.scroll -= steps as f32;
        steps
    }

    /// Consume this frame's press edges. Call once at the end of each frame.
    pub fn end_frame(&mut self) {
        self.just_pressed.clear();
        self.mouse_just_pressed.clear();
    }

    /// winit does not synthesize key-release events for keys held when focus
    /// is lost; stale state must be dropped on focus transitions.
    pub fn clear(&mut self) {
        self.pressed.clear();
        self.just_pressed.clear();
        self.mouse_held.clear();
        self.mouse_just_pressed.clear();
        self.mouse_delta = (0.0, 0.0);
        self.scroll = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode as K;

    #[test]
    fn key_pressed_fires_once_per_press_edge() {
        let mut input = InputState::default();
        input.set_key(K::KeyF, true);
        assert!(input.key_pressed(K::KeyF));
        assert!(input.is_down(K::KeyF));
        input.end_frame();
        assert!(!input.key_pressed(K::KeyF), "edge consumed");
        assert!(input.is_down(K::KeyF), "still held");
        input.set_key(K::KeyF, true); // OS key-repeat while held
        assert!(!input.key_pressed(K::KeyF), "repeat is not a new edge");
        input.set_key(K::KeyF, false);
        input.set_key(K::KeyF, true);
        assert!(input.key_pressed(K::KeyF), "release + press = new edge");
    }

    #[test]
    fn mouse_buttons_track_edges_and_held_state() {
        let mut input = InputState::default();
        input.set_mouse_button(MouseButton::Left, true);
        assert!(input.mouse_pressed(MouseButton::Left));
        assert!(input.mouse_down(MouseButton::Left));
        assert!(!input.mouse_pressed(MouseButton::Right));
        input.end_frame();
        assert!(!input.mouse_pressed(MouseButton::Left));
        assert!(input.mouse_down(MouseButton::Left));
        input.set_mouse_button(MouseButton::Left, false);
        assert!(!input.mouse_down(MouseButton::Left));
    }

    #[test]
    fn scroll_accumulates_whole_steps_and_keeps_remainder() {
        let mut input = InputState::default();
        input.accumulate_scroll(0.6);
        assert_eq!(input.take_scroll_steps(), 0);
        input.accumulate_scroll(0.6); // 0.6 remainder + 0.6 = 1.2
        assert_eq!(input.take_scroll_steps(), 1);
        input.accumulate_scroll(-2.5);
        assert_eq!(input.take_scroll_steps(), -2);
    }

    #[test]
    fn clear_drops_everything() {
        let mut input = InputState::default();
        input.set_key(K::KeyW, true);
        input.set_mouse_button(MouseButton::Right, true);
        input.accumulate_mouse(3.0, 4.0);
        input.accumulate_scroll(2.0);
        input.clear();
        assert!(!input.is_down(K::KeyW));
        assert!(!input.key_pressed(K::KeyW));
        assert!(!input.mouse_down(MouseButton::Right));
        assert_eq!(input.take_mouse_delta(), (0.0, 0.0));
        assert_eq!(input.take_scroll_steps(), 0);
    }
}
