// CampSystem — keeps the world's resident enemy populations alive
// (Phase 7.5). Each camp is `count` slots at deterministic golden-angle
// positions; a slot whose entity died refills after `respawn_seconds`.
// No-op without an ActiveChapter resource (networked display clients).

use super::chapter::{camp_slot_pos, ActiveChapter};
use engine_app::scheduler::System;
use engine_core::prefab::spawn_prefab;
use engine_core::traits::{Resources, SpawnContext};
use engine_core::World;
use glam::Vec3;
use hecs::Entity;

struct Slot {
    entity: Option<Entity>,
    /// Seconds until this slot refills; 0 with no entity = spawn now.
    respawn_in: f32,
}

pub struct CampSystem {
    /// slots[camp][slot] — built lazily from the chapter definition.
    slots: Vec<Vec<Slot>>,
    initialized: bool,
}

impl CampSystem {
    pub fn new() -> Self {
        Self { slots: Vec::new(), initialized: false }
    }
}

impl System for CampSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        // (prefab, position, camp index, slot index) for slots due to spawn.
        let due: Vec<(String, Vec3, usize, usize)> = {
            let Some(chapter) = resources.get::<ActiveChapter>() else { return };
            if !self.initialized {
                self.initialized = true;
                self.slots = chapter
                    .def
                    .camps
                    .iter()
                    .map(|c| (0..c.count).map(|_| Slot { entity: None, respawn_in: 0.0 }).collect())
                    .collect();
            }

            let mut due = Vec::new();
            for (ci, camp) in chapter.def.camps.iter().enumerate() {
                for (si, slot) in self.slots[ci].iter_mut().enumerate() {
                    if let Some(entity) = slot.entity {
                        if !world.contains(entity) {
                            slot.entity = None;
                            slot.respawn_in = camp.respawn_seconds;
                        }
                        continue;
                    }
                    slot.respawn_in = (slot.respawn_in - delta).max(0.0);
                    if slot.respawn_in == 0.0 {
                        due.push((camp.prefab.clone(), camp_slot_pos(camp, si), ci, si));
                    }
                }
            }
            due
        };

        for (prefab, pos, ci, si) in due {
            match spawn_prefab(&prefab, pos, &mut SpawnContext { world, resources }) {
                Ok(entity) => self.slots[ci][si].entity = Some(entity),
                Err(e) => log::error!("camp spawn '{prefab}' failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chapter::{CampDef, ChapterDef, SpawnConfig};
    use engine_core::prefab::{register_core_components, ComponentRegistry, PrefabDef, PrefabLibrary};

    fn camp_resources(respawn_seconds: f32) -> Resources {
        let mut registry = ComponentRegistry::new();
        register_core_components(&mut registry);
        let mut library = PrefabLibrary::new();
        library.insert(
            "dummy",
            ron::from_str::<PrefabDef>(r#"(components: { "Transform": () })"#).unwrap(),
        );
        let chapter = ActiveChapter {
            def: ChapterDef {
                name: "test".into(),
                spawning: SpawnConfig { max_alive: 10, waves: vec![] },
                initial_spawns: vec![],
                camps: vec![CampDef {
                    prefab: "dummy".into(),
                    center: Vec3::ZERO,
                    radius: 2.0,
                    count: 1,
                    respawn_seconds,
                }],
            },
            elapsed: 0.0,
            wave_timers: vec![],
            spawn_angle: 0.0,
            started: true,
        };
        let mut resources = Resources::new();
        resources.insert(registry);
        resources.insert(library);
        resources.insert(chapter);
        resources
    }

    #[test]
    fn camp_populates_on_first_tick_and_respawns_after_timer() {
        let mut world = World::new();
        let mut resources = camp_resources(0.5);
        let mut system = CampSystem::new();

        system.run(&mut world, &mut resources, 1.0 / 60.0);
        assert_eq!(world.len(), 1, "camp fills immediately");

        // Kill the resident.
        let victim = world.iter().next().unwrap().entity();
        world.despawn(victim).unwrap();

        system.run(&mut world, &mut resources, 1.0 / 60.0);
        assert_eq!(world.len(), 0, "death starts the respawn timer");
        system.run(&mut world, &mut resources, 0.3);
        assert_eq!(world.len(), 0, "0.3 s elapsed of 0.5");
        system.run(&mut world, &mut resources, 0.3);
        assert_eq!(world.len(), 1, "slot refilled after the timer");
    }
}
