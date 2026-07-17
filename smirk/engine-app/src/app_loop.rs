// ApplicationHandler impl — winit event loop wired into App::tick()

use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};
use crate::app::App;
use crate::config::{Resolution, WindowConfig, reload_config, save_window_config};
use crate::input::{KeyboardState, MouseState};
use crate::winit_processor::WinitEventProcessor;

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Build WindowAttributes from config (falls back to defaults if not set).
        let attrs = if let Some(cfg) = self.resources.get::<WindowConfig>().cloned() {
            // Resolve resolution: Auto queries the primary monitor's native size.
            let (w, h) = match cfg.resolution {
                Resolution::Fixed(w, h) => (w, h),
                Resolution::Auto => event_loop
                    .primary_monitor()
                    .map(|m| { let s = m.size(); (s.width, s.height) })
                    .unwrap_or((1280, 720)),
            };

            let fullscreen = crate::config::resolve_fullscreen(
                &cfg.mode,
                &cfg.resolution,
                event_loop.primary_monitor(),
            );

            Window::default_attributes()
                .with_title(cfg.title)
                .with_inner_size(winit::dpi::PhysicalSize::new(w, h))
                .with_fullscreen(fullscreen)
        } else {
            Window::default_attributes()
        };

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create window"),
        );
        // Store the window in Resources so systems can call set_fullscreen, set_title, etc.
        self.resources.insert(window.clone());
        self.last_tick = Instant::now();
        for f in self.on_init.drain(..) {
            f(&window, &mut self.resources);
        }
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Forward to UI subsystems (e.g. egui) before game input handling.
        // If a subsystem consumes an input event, skip game handling for that event.
        let ui_consumed = {
            let window = self.window.clone();
            match (window, self.resources.get_mut::<WinitEventProcessor>()) { (Some(w), Some(proc)) => {
                proc.process(&w, &event)
            } _ => { false }}
        };
        let is_game_input = matches!(event,
            WindowEvent::KeyboardInput { .. } | WindowEvent::MouseInput { .. }
                | WindowEvent::CursorMoved { .. } | WindowEvent::MouseWheel { .. }
        );
        if ui_consumed && is_game_input { return; }

        match event {
            WindowEvent::CloseRequested => {
                // Persist current WindowConfig back to disk so runtime changes survive restarts.
                if let (Some(path), Some(cfg)) = (
                    &self.config_path,
                    self.resources.get::<WindowConfig>().cloned(),
                )
                    && let Err(e) = save_window_config(std::path::Path::new(path), &cfg) {
                        log::warn!("failed to persist config to {path}: {e}");
                    }
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key
                    && let Some(kb) = self.resources.get_mut::<KeyboardState>() {
                        match event.state {
                            ElementState::Pressed  => kb.press(code),
                            ElementState::Released => kb.release(code),
                        }
                    }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(mouse) = self.resources.get_mut::<MouseState>() {
                    match state {
                        ElementState::Pressed  => mouse.press(button),
                        ElementState::Released => mouse.release(button),
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(mouse) = self.resources.get_mut::<MouseState>() {
                    mouse.move_to(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(mouse) = self.resources.get_mut::<MouseState>() {
                    let lines = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                        winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 120.0,
                    };
                    mouse.add_wheel(lines);
                }
            }
            WindowEvent::Focused(false) => {
                if let Some(kb) = self.resources.get_mut::<KeyboardState>() {
                    kb.clear();
                }
                if let Some(mouse) = self.resources.get_mut::<MouseState>() {
                    mouse.clear();
                }
            }
            WindowEvent::Resized(size) => {
                for f in &mut self.on_resize {
                    f(size.width, size.height, &mut self.resources);
                }
            }
            WindowEvent::RedrawRequested => {
                // Hot-reload: drain all pending file events, re-parse and apply on any change.
                let config_changed = self.config_watcher.as_ref()
                    .map(|(_, rx)| rx.try_iter().any(|r| r.is_ok()))
                    .unwrap_or(false);
                if config_changed
                    && let Some(path) = &self.config_path.clone() {
                        if let Some(new_cfg) = reload_config(std::path::Path::new(path)) {
                            if let Some(w) = &self.window {
                                w.set_title(&new_cfg.title);
                                if let Resolution::Fixed(width, height) = new_cfg.resolution {
                                    let _ = w.request_inner_size(
                                        winit::dpi::PhysicalSize::new(width, height),
                                    );
                                }
                            }
                            self.resources.insert(new_cfg);
                        } else {
                            log::warn!("hot-reload parse error for {path}");
                        }
                    }

                let now   = Instant::now();
                let delta = now.duration_since(self.last_tick).as_secs_f32().min(0.1);
                self.last_tick = now;
                self.tick(delta);

                // Frame limiter: park the event loop until the per-frame budget
                // elapses (see about_to_wait) rather than blocking-sleeping the
                // winit thread — input keeps pumping while we wait.
                // None max_fps = resolve from the current monitor's refresh rate
                // on-the-fly (not stored in config).
                let max_fps = self.resources.get::<WindowConfig>().and_then(|c| c.max_fps)
                    .or_else(|| self.window.as_ref()
                        .and_then(|w| w.current_monitor())
                        .and_then(|m| m.refresh_rate_millihertz())
                        .map(|mhz| (mhz / 1000).max(30)));
                self.next_frame = next_frame_deadline(now, max_fps);
            }
            _ => {}
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        // The WaitUntil deadline fired: drive the next frame's redraw.
        if let StartCause::ResumeTimeReached { .. } = cause
            && let Some(w) = &self.window { w.request_redraw(); }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.next_frame {
            // Cap resolved: sleep the OS timer to the deadline, waking early for
            // any input event so it is processed without a frame of latency.
            Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            // No cap: redraw as fast as the loop allows.
            None => {
                if let Some(w) = &self.window { w.request_redraw(); }
                event_loop.set_control_flow(ControlFlow::Poll);
            }
        }
    }
}

/// Next-frame deadline for the WaitUntil limiter: the tick's start instant plus
/// the per-frame budget. `None` (no cap resolved) means redraw as fast as the
/// event loop allows. Because deadlines are anchored to each tick's start, a run
/// with no overruns produces exactly `max_fps` ticks per wall-clock second.
fn next_frame_deadline(tick_start: Instant, max_fps: Option<u32>) -> Option<Instant> {
    max_fps.map(|fps| tick_start + Duration::from_secs_f64(1.0 / fps as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_fps_has_no_deadline() {
        assert!(next_frame_deadline(Instant::now(), None).is_none());
    }

    #[test]
    fn deadline_cadence_matches_max_fps() {
        // Simulate an overrun-free run: each frame's tick starts at the previous
        // frame's deadline. Every frame must be paced to exactly one budget of
        // 1/max_fps, and the count over one wall-clock second must land on
        // max_fps (±1 tick from nanosecond budget rounding).
        for fps in [30u32, 60, 144] {
            let budget = Duration::from_secs_f64(1.0 / fps as f64);
            let start = Instant::now();
            let one_second = start + Duration::from_secs(1);
            let mut tick_start = start;
            let mut ticks = 0u32;
            while let Some(deadline) = next_frame_deadline(tick_start, Some(fps)) {
                assert_eq!(deadline - tick_start, budget, "per-frame budget for {fps} fps");
                if deadline > one_second { break; }
                ticks += 1;
                tick_start = deadline;
            }
            assert!(ticks.abs_diff(fps) <= 1, "cadence for {fps} fps: got {ticks}");
        }
    }
}
