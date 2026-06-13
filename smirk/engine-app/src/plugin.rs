// Plugin — the single extension point for engine subsystems, game modules,
// and chapters. Everything that wires into the App goes through this trait.
//
// Usage (in game/src/main.rs):
//
//   App::new()
//       .configure("assets/config/engine.ron")
//       .add_plugin(RenderPlugin)
//       .add_plugin(PhysicsPlugin)
//       .add_plugin(CoreGamePlugin)
//       .run();
//
// Plugins apply in add_plugin call order. Within a frame, execution order is
// governed entirely by Phase + SystemOrder, so plugin order only matters for
// startup concerns (e.g. on_window_ready callbacks fire in registration order).

use crate::app::App;

pub trait Plugin {
    /// Configure the App: insert resources, add systems, register components,
    /// load prefabs. Called once, before the event loop starts.
    fn build(&self, app: &mut App);

    /// Name used in startup logs. Defaults to the type name.
    fn name(&self) -> &'static str
    where
        Self: Sized,
    {
        std::any::type_name::<Self>()
    }
}
