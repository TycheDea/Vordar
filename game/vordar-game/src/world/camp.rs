// CampSystem — keeps the world's resident enemy populations alive. Each
// camp is `count` slots at deterministic golden-angle positions; a slot
// whose entity died refills after `respawn_seconds`. No-op without a
// ChapterDef resource (networked display clients).

use super::chapter::{camp_slot_pos, ChapterDef};
use engine_app::scheduler::System;
use engine_core::prefab::spawn_prefab;
use engine_core::traits::{Resources, SpawnQueue};
use engine_core::World;
use glam::Vec3;
use std::collections::HashSet;

/// Marks an entity as occupying a camp's slot. CampSystem finds a slot's
/// occupant by querying for this component rather than caching an Entity id,
/// so a slot reads as free the instant its entity is gone, regardless of
/// despawn order elsewhere in the frame.
pub struct CampMember {
    pub camp: usize,
    pub slot: usize,
}

struct Slot {
    /// Was this slot's entity alive as of the last tick — distinguishes
    /// "just died" (start the respawn timer) from "already empty" (keep
    /// counting down).
    occupied: bool,
    /// Seconds until this slot refills; 0 with no entity = spawn now.
    respawn_in: f32,
}

pub struct CampSystem {
    /// slots[camp][slot] — built lazily from the chapter definition.
    slots: Vec<Vec<Slot>>,
    initialized: bool,
}

impl Default for CampSystem {
    fn default() -> Self {
        Self::new()
    }
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
            let Some(chapter) = resources.get::<ChapterDef>() else { return };
            if !self.initialized {
                self.initialized = true;
                self.slots = chapter
                    .camps
                    .iter()
                    .map(|c| (0..c.count).map(|_| Slot { occupied: false, respawn_in: 0.0 }).collect())
                    .collect();
            }

            let occupied: HashSet<(usize, usize)> =
                world.query::<&CampMember>().iter().map(|m| (m.camp, m.slot)).collect();

            let mut due = Vec::new();
            for (ci, camp) in chapter.camps.iter().enumerate() {
                for (si, slot) in self.slots[ci].iter_mut().enumerate() {
                    if occupied.contains(&(ci, si)) {
                        slot.occupied = true;
                        continue;
                    }
                    if slot.occupied {
                        slot.occupied = false;
                        slot.respawn_in = camp.respawn_seconds;
                        continue;
                    }
                    slot.respawn_in = (slot.respawn_in - delta).max(0.0);
                    if slot.respawn_in == 0.0 {
                        due.push((camp.prefab.clone(), camp_slot_pos(camp, si), ci, si));
                        // Mark filled the instant the spawn is queued (not
                        // when the query later confirms it) — the entity
                        // isn't visible in the world until next SpawnFlush,
                        // but the slot must read as taken before then too.
                        slot.occupied = true;
                    }
                }
            }
            due
        };

        for (prefab, pos, ci, si) in due {
            resources.expect_mut::<SpawnQueue>().push(move |ctx| {
                match spawn_prefab(&prefab, pos, ctx) {
                    Ok(entity) => { let _ = ctx.world.insert_one(entity, CampMember { camp: ci, slot: si }); }
                    Err(e) => log::error!("camp spawn '{prefab}' failed: {e}"),
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chapter::{CampDef, ChapterDef};
    use engine_core::prefab::{register_core_components, ComponentRegistry, PrefabDef, PrefabLibrary};
    use engine_core::traits::SpawnContext;
    use hecs::Entity;

    fn camp_resources(respawn_seconds: f32) -> Resources {
        let mut registry = ComponentRegistry::new();
        register_core_components(&mut registry);
        let mut library = PrefabLibrary::new();
        library.insert(
            "dummy",
            ron::from_str::<PrefabDef>(r#"(components: { "Transform": () })"#).unwrap(),
        );
        let chapter = ChapterDef {
            name: "test".into(),
            initial_spawns: vec![],
            camps: vec![CampDef {
                prefab: "dummy".into(),
                center: Vec3::ZERO,
                radius: 2.0,
                count: 1,
                respawn_seconds,
            }],
        };
        let mut resources = Resources::new();
        resources.insert(registry);
        resources.insert(library);
        resources.insert(chapter);
        resources.insert(SpawnQueue::new());
        resources
    }

    /// Runs the system, then drains SpawnQueue the way engine-app's
    /// SpawnFlushSystem does — CampSystem now queues instead of spawning
    /// directly, so a tick isn't complete until the queue is flushed.
    fn tick(system: &mut CampSystem, world: &mut World, resources: &mut Resources, delta: f32) {
        system.run(world, resources, delta);
        let fns: Vec<_> = resources.get_mut::<SpawnQueue>().unwrap().0.drain(..).collect();
        for f in fns {
            f(&mut SpawnContext { world, resources });
        }
    }

    #[test]
    fn camp_run_only_queues_the_spawn_never_mutates_the_world_directly() {
        let mut world = World::new();
        let mut resources = camp_resources(0.5);
        let mut system = CampSystem::new();

        system.run(&mut world, &mut resources, 1.0 / 60.0);
        assert_eq!(world.len(), 0, "CampSystem must not spawn directly — only queue");
        assert_eq!(resources.get::<SpawnQueue>().unwrap().0.len(), 1, "spawn queued for the flush phase");
    }

    #[test]
    fn camp_populates_on_first_tick_and_respawns_after_timer() {
        let mut world = World::new();
        let mut resources = camp_resources(0.5);
        let mut system = CampSystem::new();

        tick(&mut system, &mut world, &mut resources, 1.0 / 60.0);
        assert_eq!(world.len(), 1, "camp fills immediately");

        // Kill the resident, identified by its CampMember slot rather than a
        // cached Entity id.
        let victim: Entity = world.query::<(Entity, &CampMember)>().iter().next().unwrap().0;
        world.despawn(victim).unwrap();

        tick(&mut system, &mut world, &mut resources, 1.0 / 60.0);
        assert_eq!(world.len(), 0, "death starts the respawn timer");
        tick(&mut system, &mut world, &mut resources, 0.3);
        assert_eq!(world.len(), 0, "0.3 s elapsed of 0.5");
        tick(&mut system, &mut world, &mut resources, 0.3);
        assert_eq!(world.len(), 1, "slot refilled after the timer");
    }
}
