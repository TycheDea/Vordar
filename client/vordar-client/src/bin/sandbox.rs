// Sandbox — single-process build: client presentation + shared simulation,
// no networking. The fast-iteration harness between networking milestones.

use engine_app::app::App;
use engine_app::prefab_plugin::PrefabPlugin;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::prefab::queue_prefab_spawn;
use engine_core::traits::Resources;
use engine_core::World;
use engine_physics::PhysicsPlugin;
use engine_renderer::RenderPlugin;
use vordar_client::ClientPlugin;
use vordar_game::CoreGamePlugin;

/// Spawns the local player once. Players are not chapter content — whoever
/// has authority spawns them (here: this process; online: the server).
struct SpawnPlayerSystem {
    done: bool,
}

impl System for SpawnPlayerSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        if self.done {
            return;
        }
        self.done = true;
        // The player: a Human ravager. Phase C — BodyComposeSystem gives it the
        // rigged human.glb mesh + locomotion, so WASD drives a real skinned
        // character that idles / runs / turns and swings on a cast.
        queue_prefab_spawn(resources, "ravager", glam::Vec3::ZERO);
    }
}

fn main() {
    let mut app = App::new();
    app.configure("content/config/engine.ron")
        .add_plugin(RenderPlugin)
        .add_plugin(PhysicsPlugin)
        .add_plugin(PrefabPlugin)
        .add_plugin(CoreGamePlugin);
    vordar_game::chapter::ChapterRegistry::new(vec![chapter_01::module(), chapter_02::module()])
        .install("chapter01", &mut app)
        .expect("chapter01 must be linked");
    app.add_plugin(ClientPlugin)
        .add_system(SpawnPlayerSystem { done: false }, Phase::PreUpdate, SystemOrder::First)
        .run()
}
