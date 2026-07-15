// KeyboardState / MouseState — raw input state, updated by the event loop.
//
// Inserted into Resources by App::new(). Updated in ApplicationHandler::window_event
// before any system runs. Systems read them via resources.get::<KeyboardState>().
// Events consumed by UI (egui) never reach these — see app_loop's ui_consumed gate.
//
// just_pressed/just_released record raw press/release events independently of
// the level (`pressed`) state, so a press+release that both land within one
// frame's event batch still yields one edge instead of cancelling to nothing.
// InputEdgeFlushSystem drains them once per fixed Input tick (SystemOrder::Last,
// after every Input-phase consumer has had a chance to observe them), so a
// multi-step catch-up frame doesn't replay the same edge into later steps.

use std::collections::HashSet;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

pub struct KeyboardState {
    pressed:       HashSet<KeyCode>,
    just_pressed:  HashSet<KeyCode>,
    just_released: HashSet<KeyCode>,
}

impl KeyboardState {
    pub fn new() -> Self {
        Self { pressed: HashSet::new(), just_pressed: HashSet::new(), just_released: HashSet::new() }
    }

    pub fn is_pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    /// True if `key` transitioned up→down since the last Input-tick drain.
    pub fn just_pressed(&self, key: KeyCode) -> bool {
        self.just_pressed.contains(&key)
    }

    /// True if `key` transitioned down→up since the last Input-tick drain.
    pub fn just_released(&self, key: KeyCode) -> bool {
        self.just_released.contains(&key)
    }

    /// Public so headless tests in other crates can drive a real held key
    /// through the normal `KeyboardState` → movement-system path instead of
    /// reimplementing input handling.
    pub fn press(&mut self, key: KeyCode) {
        self.pressed.insert(key);
        self.just_pressed.insert(key);
    }

    pub(crate) fn release(&mut self, key: KeyCode) {
        self.pressed.remove(&key);
        self.just_released.insert(key);
    }

    pub(crate) fn clear(&mut self) {
        self.pressed.clear();
        self.just_pressed.clear();
        self.just_released.clear();
    }

    pub(crate) fn drain_edges(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
    }
}

pub struct MouseState {
    pressed:       HashSet<MouseButton>,
    just_pressed:  HashSet<MouseButton>,
    just_released: HashSet<MouseButton>,
    /// Cursor position in physical pixels; None until the cursor first enters.
    cursor: Option<(f32, f32)>,
    /// Wheel scroll accumulated since the last `take_wheel` (in lines).
    wheel: f32,
}

impl MouseState {
    pub fn new() -> Self {
        Self {
            pressed:       HashSet::new(),
            just_pressed:  HashSet::new(),
            just_released: HashSet::new(),
            cursor:        None,
            wheel:         0.0,
        }
    }

    pub fn is_pressed(&self, button: MouseButton) -> bool {
        self.pressed.contains(&button)
    }

    /// True if `button` transitioned up→down since the last Input-tick drain.
    pub fn just_pressed(&self, button: MouseButton) -> bool {
        self.just_pressed.contains(&button)
    }

    /// True if `button` transitioned down→up since the last Input-tick drain.
    pub fn just_released(&self, button: MouseButton) -> bool {
        self.just_released.contains(&button)
    }

    pub fn cursor(&self) -> Option<(f32, f32)> {
        self.cursor
    }

    /// Drain the accumulated wheel delta. One consumer per frame: whoever
    /// owns zooming calls this once and gets everything since the last call.
    pub fn take_wheel(&mut self) -> f32 {
        std::mem::take(&mut self.wheel)
    }

    pub(crate) fn press(&mut self, button: MouseButton) {
        self.pressed.insert(button);
        self.just_pressed.insert(button);
    }

    pub(crate) fn release(&mut self, button: MouseButton) {
        self.pressed.remove(&button);
        self.just_released.insert(button);
    }

    pub(crate) fn move_to(&mut self, x: f32, y: f32) { self.cursor = Some((x, y)); }
    pub(crate) fn add_wheel(&mut self, lines: f32)   { self.wheel += lines; }

    pub(crate) fn clear(&mut self) {
        self.pressed.clear();
        self.just_pressed.clear();
        self.just_released.clear();
        self.wheel = 0.0;
    }

    pub(crate) fn drain_edges(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
    }
}

/// Drains KeyboardState/MouseState edge sets once per fixed Input tick.
/// Registered at Phase::Input, SystemOrder::Last by App::new() so every
/// Input-phase consumer (ability casts, etc.) observes an edge before it's
/// cleared for the next step.
pub(crate) struct InputEdgeFlushSystem;

impl crate::scheduler::System for InputEdgeFlushSystem {
    fn run(&mut self, _world: &mut engine_core::World, resources: &mut engine_core::traits::Resources, _delta: f32) {
        if let Some(kb) = resources.get_mut::<KeyboardState>() { kb.drain_edges(); }
        if let Some(mouse) = resources.get_mut::<MouseState>() { mouse.drain_edges(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_release_within_one_frame_yields_one_edge_until_drained() {
        let mut kb = KeyboardState::new();
        // Both events land before any tick observes them (a fast tap during
        // a frame dip) — level state cancels out, but the edges must not.
        kb.press(KeyCode::KeyQ);
        kb.release(KeyCode::KeyQ);

        assert!(!kb.is_pressed(KeyCode::KeyQ), "level state should cancel out");
        assert!(kb.just_pressed(KeyCode::KeyQ), "the press edge must survive to the next tick");
        assert!(kb.just_released(KeyCode::KeyQ), "the release edge must survive to the next tick");

        kb.drain_edges();
        assert!(!kb.just_pressed(KeyCode::KeyQ), "drained edges must not leak into the next tick");
        assert!(!kb.just_released(KeyCode::KeyQ));
    }

    #[test]
    fn drain_edges_prevents_replay_across_catch_up_steps() {
        // A multi-step catch-up frame runs Phase::Input more than once with
        // no new real events in between; the edge must fire on the first
        // step only, not on every step of the frame.
        let mut kb = KeyboardState::new();
        kb.press(KeyCode::KeyE);

        assert!(kb.just_pressed(KeyCode::KeyE));
        kb.drain_edges(); // end of step 1's Input phase
        assert!(!kb.just_pressed(KeyCode::KeyE), "step 2 must not replay step 1's edge");
    }

    #[test]
    fn input_edge_flush_system_drains_both_resources() {
        use crate::scheduler::System;
        use engine_core::traits::Resources;
        use engine_core::World;

        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(KeyboardState::new());
        resources.insert(MouseState::new());
        resources.get_mut::<KeyboardState>().unwrap().press(KeyCode::KeyQ);
        resources.get_mut::<MouseState>().unwrap().press(MouseButton::Left);

        InputEdgeFlushSystem.run(&mut world, &mut resources, 1.0 / 60.0);

        assert!(!resources.get::<KeyboardState>().unwrap().just_pressed(KeyCode::KeyQ));
        assert!(!resources.get::<MouseState>().unwrap().just_pressed(MouseButton::Left));
    }
}
