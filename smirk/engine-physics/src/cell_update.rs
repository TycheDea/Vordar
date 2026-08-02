// CellUpdateSystem — keeps SpatialGrid in sync with entity positions.
//
// Runs Phase::Collision, First — right before broadphase reads the grid, at
// the full collision rate: collision needs the grid no more than one frame
// stale, and PostUpdate's snapshot-rate cadence would leave it up to 100 ms
// stale.
//
// The grid persists across ticks; each entity's footprint is updated by diffing
// its new cells against CellOccupant.cells (its footprint last tick) and only
// removing/inserting on difference. An entity that did not change cells costs
// nothing but the recompute — no allocator traffic, no grid mutation. Despawns
// are removed from the grid by DespawnFlush (engine-app/src/flush.rs) using the
// same CellOccupant.cells record.
//
// Per frame, for each entity with Transform + Hitbox + CellOccupant:
//   1. Compute the cells it now occupies.
//   2. Remove it from cells it left; insert it into cells it entered.
//   3. Record the new footprint in CellOccupant.

use engine_app::scheduler::System;
use engine_core::components::{CellOccupant, CollisionShape, GridCell, Hitbox, Transform};
use engine_core::spatial::SpatialGrid;
use engine_core::traits::Resources;
use engine_core::World;
use hecs::Entity;

pub struct CellUpdateSystem {
    scratch: Vec<GridCell>,
}

impl Default for CellUpdateSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl CellUpdateSystem {
    pub fn new() -> Self { Self { scratch: Vec::new() } }
}

impl System for CellUpdateSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let grid = resources.expect_mut::<SpatialGrid>();

        let cell_size = grid.cell_size();

        // Single pass: query only borrows Transform + Hitbox, so world.get::<&mut CellOccupant>
        // is safe during iteration — hecs uses per-storage locks and CellOccupant is unlocked.
        // Entities without CellOccupant are skipped (world.get returns Err).
        for (entity, transform, hitbox) in world.query::<(Entity, &Transform, &Hitbox)>().iter() {
            let mut occupant = match world.get::<&mut CellOccupant>(entity) {
                Ok(o) => o,
                Err(_) => continue,
            };

            self.scratch.clear();
            cells_for_hitbox_into(transform, hitbox, cell_size, &mut self.scratch);

            // Footprints are 1–4 cells, so linear membership checks are cheaper
            // than building sets; only differences touch the grid.
            for &cell in &occupant.cells {
                if !self.scratch.contains(&cell) {
                    grid.remove(cell, entity);
                }
            }
            for &cell in &self.scratch {
                if !occupant.cells.contains(&cell) {
                    grid.insert_in_cell(entity, cell);
                }
            }

            occupant.cells.clear();
            occupant.cells.extend_from_slice(&self.scratch);
        }
    }
}

fn cells_for_hitbox_into(transform: &Transform, hitbox: &Hitbox, cell_size: f32, out: &mut Vec<GridCell>) {
    let (half_x, half_z) = match hitbox.shape {
        CollisionShape::Aabb { half_extents } => (half_extents.x, half_extents.z),
        CollisionShape::Sphere { radius }     => (radius, radius),
    };

    let pos = transform.position;
    let min_col = ((pos.x - half_x) / cell_size).floor() as i32;
    let max_col = ((pos.x + half_x) / cell_size).floor() as i32;
    let min_row = ((pos.z - half_z) / cell_size).floor() as i32;
    let max_row = ((pos.z + half_z) / cell_size).floor() as i32;

    for col in min_col..=max_col {
        for row in min_row..=max_row {
            out.push(GridCell { col, row });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    const CELL: f32 = 10.0;

    fn setup() -> (World, Resources) {
        let mut resources = Resources::new();
        resources.insert(SpatialGrid::new(CELL));
        (World::new(), resources)
    }

    fn spawn_point(world: &mut World, pos: Vec3) -> Entity {
        world.spawn((
            Transform::new(pos),
            Hitbox { shape: CollisionShape::Sphere { radius: 0.5 } },
            CellOccupant { cells: Default::default() },
        ))
    }

    fn cell_of(pos: Vec3) -> GridCell {
        GridCell { col: (pos.x / CELL).floor() as i32, row: (pos.z / CELL).floor() as i32 }
    }

    #[test]
    fn entity_crossing_cell_boundary_moves_in_grid() {
        let (mut world, mut resources) = setup();
        let e = spawn_point(&mut world, Vec3::new(5.0, 0.0, 5.0));
        let mut sys = CellUpdateSystem::new();
        sys.run(&mut world, &mut resources, 0.0);

        let old = cell_of(Vec3::new(5.0, 0.0, 5.0));
        assert_eq!(resources.get::<SpatialGrid>().unwrap().query_cell(old), &[e]);

        world.get::<&mut Transform>(e).unwrap().position = Vec3::new(15.0, 0.0, 5.0);
        sys.run(&mut world, &mut resources, 0.0);

        let new = cell_of(Vec3::new(15.0, 0.0, 5.0));
        let grid = resources.get::<SpatialGrid>().unwrap();
        assert!(grid.query_cell(old).is_empty(), "left cell must be vacated");
        assert_eq!(grid.query_cell(new), &[e], "entity present in the new cell exactly once");
    }

    #[test]
    fn stationary_entity_is_present_exactly_once_across_ticks() {
        let (mut world, mut resources) = setup();
        let e = spawn_point(&mut world, Vec3::new(3.0, 0.0, 3.0));
        let cell = cell_of(Vec3::new(3.0, 0.0, 3.0));
        let mut sys = CellUpdateSystem::new();

        // Five rebuilds of a still entity must leave one grid entry, not five —
        // the incremental diff never re-inserts an unchanged footprint.
        for _ in 0..5 {
            sys.run(&mut world, &mut resources, 0.0);
        }

        assert_eq!(resources.get::<SpatialGrid>().unwrap().query_cell(cell), &[e]);
    }
}
