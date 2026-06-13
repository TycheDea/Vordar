// UiLayers — game-registered egui draw callbacks.
//
// The engine owns the egui frame (menu, dev overlay); everything game-flavored
// (minimap, action bar, ...) is drawn by callbacks the game registers here.
// RenderSystem invokes them inside its egui pass each display frame with
// read-only access to Resources: publish UI state from your systems, read it
// in the callback. Register at startup (a plugin's build) — layers added
// while a frame is being drawn are dropped.

use engine_core::traits::Resources;

// Send + Sync because App::insert_resource requires it (apps are built on
// arbitrary threads). Plain fn items qualify automatically.
type UiLayerFn = Box<dyn FnMut(&egui::Context, &Resources) + Send + Sync>;

#[derive(Default)]
pub struct UiLayers {
    pub(crate) layers: Vec<UiLayerFn>,
}

impl UiLayers {
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Add a draw callback. Layers draw in registration order.
    pub fn add(&mut self, f: impl FnMut(&egui::Context, &Resources) + Send + Sync + 'static) {
        self.layers.push(Box::new(f));
    }
}
