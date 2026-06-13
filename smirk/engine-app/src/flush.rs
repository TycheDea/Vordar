// Flush systems — engine-provided, registered automatically by App::new()

use engine_core::components::CellOccupant;
use engine_core::spatial::SpatialGrid;
use crate::events::EventBus;
use crate::scheduler::System;
use engine_core::traits::{DespawnQueue, Resources, SpawnContext, SpawnQueue};
use engine_core::World;

pub(crate) struct ClearEventsSystem;

impl System for ClearEventsSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        resources.get_mut::<EventBus>().unwrap().clear();
    }
}

pub(crate) struct SpawnFlushSystem;

impl System for SpawnFlushSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let queue = resources.get_mut::<SpawnQueue>().unwrap();
        let fns: Vec<_> = queue.0.drain(..).collect();
        for f in fns { f(&mut SpawnContext { world, resources }); }
    }
}

pub(crate) struct DespawnFlushSystem;

impl System for DespawnFlushSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {

        let pairs: Vec<_> = resources.get_mut::<DespawnQueue>().unwrap().0.drain(..).collect();
        for (entity, hook) in pairs {
            if let Some(f) = hook {
                f(&mut SpawnContext { world, resources });
            }

            // SpatialGrid belongs to the physics plugin — apps without physics
            // (e.g. the networked client) still despawn CellOccupant entities.
            if let Ok(occupant) = world.get::<&CellOccupant>(entity) {
                if let Some(grid) = resources.get_mut::<SpatialGrid>() {
                    for cell in &occupant.cells { grid.remove(*cell, entity); }
                }
            }

            world.despawn(entity).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: apps without PhysicsPlugin (networked client) have no
    // SpatialGrid resource but still despawn CellOccupant entities.
    #[test]
    fn despawn_with_cell_occupant_but_no_spatial_grid() {
        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(DespawnQueue::new());

        let entity = world.spawn((CellOccupant { cells: Default::default() },));
        resources.get_mut::<DespawnQueue>().unwrap().push(entity, None);

        DespawnFlushSystem.run(&mut world, &mut resources, 1.0 / 60.0);
        assert!(!world.contains(entity));
    }
}
