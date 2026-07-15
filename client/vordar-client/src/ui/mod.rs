// Game-owned UI: minimap + action bar, drawn through the engine's UiLayers
// seam (the engine runs the egui frame; the game registers what to draw).

pub mod action_bar;
pub mod minimap;

use engine_app::app::App;
use engine_app::scheduler::{Phase, SystemOrder};
use engine_renderer::{CameraConfig, UiLayers};

/// Wires the client UI into an app: state resources, sync systems, and the
/// UiLayers draw callbacks. Called by both ClientPlugin and NetClientPlugin.
pub fn install(app: &mut App) {
    let mut layers = UiLayers::new();
    layers.add(minimap::draw);
    layers.add(action_bar::draw);
    app.insert_resource(layers)
        .insert_resource(minimap::HudState::default())
        .insert_resource(action_bar::ActionBarState::default())
        // Presentation tuning the game owns (engine defaults would also do).
        .insert_resource(CameraConfig { min_radius: 16.0, max_radius: 55.0 })
        .add_system(minimap::HudSyncSystem, Phase::RenderSync, SystemOrder::Default)
        .add_system(action_bar::ActionBarSyncSystem, Phase::RenderSync, SystemOrder::Default);
}
