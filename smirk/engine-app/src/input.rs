// KeyboardState / MouseState — raw input state, updated by the event loop.
//
// Inserted into Resources by App::new(). Updated in ApplicationHandler::window_event
// before any system runs. Systems read them via resources.get::<KeyboardState>().
// Events consumed by UI (egui) never reach these — see app_loop's ui_consumed gate.

use std::collections::HashSet;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

pub struct KeyboardState {
    pressed: HashSet<KeyCode>,
}

impl KeyboardState {
    pub fn new() -> Self { Self { pressed: HashSet::new() } }

    pub fn is_pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    pub(crate) fn press(&mut self, key: KeyCode)   { self.pressed.insert(key); }
    pub(crate) fn release(&mut self, key: KeyCode) { self.pressed.remove(&key); }
    pub(crate) fn clear(&mut self)                 { self.pressed.clear(); }
}

pub struct MouseState {
    pressed: HashSet<MouseButton>,
    /// Cursor position in physical pixels; None until the cursor first enters.
    cursor: Option<(f32, f32)>,
    /// Wheel scroll accumulated since the last `take_wheel` (in lines).
    wheel: f32,
}

impl MouseState {
    pub fn new() -> Self {
        Self { pressed: HashSet::new(), cursor: None, wheel: 0.0 }
    }

    pub fn is_pressed(&self, button: MouseButton) -> bool {
        self.pressed.contains(&button)
    }

    pub fn cursor(&self) -> Option<(f32, f32)> {
        self.cursor
    }

    /// Drain the accumulated wheel delta. One consumer per frame: whoever
    /// owns zooming calls this once and gets everything since the last call.
    pub fn take_wheel(&mut self) -> f32 {
        std::mem::take(&mut self.wheel)
    }

    pub(crate) fn press(&mut self, button: MouseButton)   { self.pressed.insert(button); }
    pub(crate) fn release(&mut self, button: MouseButton) { self.pressed.remove(&button); }
    pub(crate) fn move_to(&mut self, x: f32, y: f32)      { self.cursor = Some((x, y)); }
    pub(crate) fn add_wheel(&mut self, lines: f32)        { self.wheel += lines; }
    pub(crate) fn clear(&mut self) {
        self.pressed.clear();
        self.wheel = 0.0;
    }
}
