// Chapter initialization — spawns initial entities on the first run.
// Players are NOT chapter content — whoever has authority spawns them
// (the sandbox locally, the server per connection).

use super::chapter::ChapterDef;
use engine_app::scheduler::System;
use engine_core::prefab::queue_prefab_spawn;
use engine_core::traits::Resources;
use engine_core::World;

/// Spawns the chapter's initial entities on the first run. Run-once latch via
/// the `done` field.
pub struct ChapterSetupSystem {
    done: bool,
}

impl Default for ChapterSetupSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl ChapterSetupSystem {
    pub fn new() -> Self { Self { done: false } }
}

impl System for ChapterSetupSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let initial = {
            let Some(chapter) = resources.get::<ChapterDef>() else {
                if !self.done {
                    log::warn!("no ChapterDef resource — nothing will spawn (add a chapter plugin)");
                    self.done = true;
                }
                return;
            };
            if self.done { return; }
            self.done = true;

            // Clone the spawn list out so the resources borrow ends before queueing.
            let mut initial: Vec<(String, glam::Vec3)> = Vec::new();
            for spawn in &chapter.initial_spawns {
                for &pos in &spawn.positions {
                    initial.push((spawn.prefab.clone(), pos));
                }
            }
            initial
        };

        for (prefab, pos) in initial {
            queue_prefab_spawn(resources, prefab, pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chapter::{ChapterDef, InitialSpawn};
    use engine_core::prefab::{register_core_components, ComponentRegistry, PrefabDef, PrefabLibrary};
    use glam::Vec3;

    fn setup_resources() -> (World, Resources) {
        let mut registry = ComponentRegistry::new();
        register_core_components(&mut registry);
        let mut library = PrefabLibrary::new();
        library.insert(
            "dummy",
            ron::from_str::<PrefabDef>(r#"(components: { "Transform": () })"#).unwrap(),
        );
        let chapter = ChapterDef {
            name: "test".into(),
            initial_spawns: vec![InitialSpawn {
                prefab: "dummy".into(),
                positions: vec![Vec3::ZERO],
            }],
            camps: vec![],
        };
        let mut resources = Resources::new();
        resources.insert(registry);
        resources.insert(library);
        resources.insert(chapter);
        resources.insert(engine_core::traits::SpawnQueue::new());
        (World::new(), resources)
    }

    #[test]
    fn chapter_setup_spawns_initial_entities_once() {
        let (mut world, mut resources) = setup_resources();
        let mut system = ChapterSetupSystem::new();

        // First run: queues the spawn
        system.run(&mut world, &mut resources, 1.0 / 60.0);
        assert_eq!(
            resources.get::<engine_core::traits::SpawnQueue>().unwrap().0.len(),
            1,
            "initial spawn queued on first run"
        );

        // Drain and complete the spawn
        let fns: Vec<_> = resources
            .get_mut::<engine_core::traits::SpawnQueue>()
            .unwrap()
            .0
            .drain(..)
            .collect();
        for f in fns {
            f(&mut engine_core::traits::SpawnContext { world: &mut world, resources: &mut resources });
        }
        assert_eq!(world.len(), 1, "entity spawned in world");

        // Second run: latch holds, no new spawns
        system.run(&mut world, &mut resources, 1.0 / 60.0);
        assert_eq!(
            resources.get::<engine_core::traits::SpawnQueue>().unwrap().0.len(),
            0,
            "run-once latch prevents re-spawn"
        );
    }
}
