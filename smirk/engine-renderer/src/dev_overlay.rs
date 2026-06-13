// Dev overlay — F3 toggles a small stats panel (top-left, egui).
//
// DevOverlaySystem handles the toggle and publishes renderer-side counters
// into DevStats; the actual drawing happens inside RenderSystem's egui frame
// (see lib.rs) because the egui context lives in RendererState.

use crate::instance::InstancePool;
use engine_app::dev_stats::DevStats;
use engine_app::input::KeyboardState;
use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;
use winit::keyboard::KeyCode;

pub struct DevOverlaySystem {
    was_f3: bool,
}

impl DevOverlaySystem {
    pub fn new() -> Self { Self { was_f3: false } }
}

impl System for DevOverlaySystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let f3 = resources.get::<KeyboardState>()
            .map(|kb| kb.is_pressed(KeyCode::F3))
            .unwrap_or(false);
        let entities  = world.len();
        let instances = resources.get::<InstancePool>().map(|p| p.used()).unwrap_or(0);

        let Some(stats) = resources.get_mut::<DevStats>() else { return; };
        if f3 && !self.was_f3 { stats.open = !stats.open; }
        self.was_f3 = f3;

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
