// Dev overlay — F3 toggles a small stats panel (top-left, egui).
//
// DevOverlaySystem handles the toggle and publishes renderer-side counters
// into DevStats; the actual drawing happens inside RenderSystem's egui pass
// because the egui context lives in RendererState.

use crate::instance::InstancePool;
use engine_app::dev_stats::DevStats;
use engine_app::input::KeyboardState;
use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;
use winit::keyboard::KeyCode;

pub struct DevOverlaySystem;

impl System for DevOverlaySystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let f3 = resources.get::<KeyboardState>()
            .map(|kb| kb.just_pressed(KeyCode::F3))
            .unwrap_or(false);
        let entities  = world.len();
        let instances = resources.get::<InstancePool>().map(|p| p.used()).unwrap_or(0);

        let Some(stats) = resources.get_mut::<DevStats>() else { return; };
        if f3 { stats.open = !stats.open; }

        if stats.open {
            stats.set("entities", entities);
            stats.set("gpu instances", instances);
        }
    }
}

/// Draw the stats panel. Non-interactable so it never steals game input.
pub(crate) fn draw_dev_overlay(ctx: &egui::Context, lines: &[(String, String)]) {
    use egui::{Align2, Area, Color32, Id, RichText, Vec2};

    Area::new(Id::new("dev_overlay"))
        .anchor(Align2::LEFT_TOP, Vec2::new(8.0, 8.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_black_alpha(170))
                .inner_margin(8.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (key, value) in lines {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{key:<16}"))
                                .color(Color32::from_rgb(150, 150, 150))
                                .monospace().size(13.0));
                            ui.label(RichText::new(value)
                                .color(Color32::WHITE)
                                .monospace().size(13.0));
                        });
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_app::input::InputEdgeFlushSystem;
    use engine_app::scheduler::{Phase, Scheduler, SystemOrder};

    #[test]
    fn fast_f3_tap_toggles_overlay_once_across_catch_up_steps() {
        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(KeyboardState::new());
        resources.insert(DevStats::new());

        let mut sched = Scheduler::new();
        sched.add(DevOverlaySystem, Phase::PostUpdate, SystemOrder::First);
        sched.add(InputEdgeFlushSystem, Phase::PostUpdate, SystemOrder::Last);
        sched.build();

        // Fast tap: both press and release land before any step observes
        // them — the drop scenario the latch-based toggle missed.
        resources.get_mut::<KeyboardState>().unwrap().press(KeyCode::F3);
        resources.get_mut::<KeyboardState>().unwrap().release(KeyCode::F3);

        sched.run_tick(&mut world, &mut resources, 3.5 / 60.0); // 3 fixed steps
        assert!(
            resources.get::<DevStats>().unwrap().open,
            "fast tap must toggle the overlay open exactly once"
        );

        // A second tick with no new input must not replay the edge.
        sched.run_tick(&mut world, &mut resources, 1.5 / 60.0);
        assert!(
            resources.get::<DevStats>().unwrap().open,
            "no new input must leave the overlay open"
        );
    }
}
