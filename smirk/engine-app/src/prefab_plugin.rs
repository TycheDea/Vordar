// PrefabPlugin — inserts the data-driven spawning machinery and registers
// loaders for all engine-core components.
//
// Game and chapter plugins then add their own pieces via the App helpers:
//   app.register_component::<MyComponent>("MyComponent")
//      .add_prefab_dir("assets/my_prefabs");

use crate::app::App;
use crate::plugin::Plugin;
use engine_core::prefab::{register_core_components, ComponentRegistry, PrefabLibrary};

pub struct PrefabPlugin;

impl Plugin for PrefabPlugin {
    fn build(&self, app: &mut App) {
        if !app.resources.contains::<ComponentRegistry>() {
            app.resources.insert(ComponentRegistry::new());
        }
        if !app.resources.contains::<PrefabLibrary>() {
            app.resources.insert(PrefabLibrary::new());
        }
        let registry = app.resources.get_mut::<ComponentRegistry>().unwrap();
        register_core_components(registry);
        log::info!("ComponentRegistry: {} components registered", registry.len());
    }
}
