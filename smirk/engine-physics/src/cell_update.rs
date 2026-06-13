// CellUpdateSystem — keeps SpatialGrid in sync with entity positions.
//
// Runs Phase::Collision, First — right before broadphase reads the grid, at
// the full collision rate (PostUpdate runs at snapshot rate on the server,
// which left the grid up to 100 ms stale for 60 Hz collision tests).
//
// Per frame:
//   1. Clear the grid.
//   2. For each entity with Transform + Hitbox + CellOccupant: compute occupied cells,
//      insert entity into those cells in SpatialGrid, record in CellOccupant.

use engine_app::scheduler::System;
use engine_core::components::{CellOccupant, CollisionShape, GridCell, Hitbox, Transform};
use engine_core::spatial::SpatialGrid;
use engine_core::traits::Resources;
use engine_core::World;
use hecs::Entity;

pub struct CellUpdateSystem {
    scratch: Vec<GridCell>,
}

impl CellUpdateSystem {
    pub fn new() -> Self { Self { scratch: Vec::new() } }
}

impl System for CellUpdateSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let grid = resources
            .get_mut::<SpatialGrid>()
            .expect("SpatialGrid not in resources");

        grid.clear();
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

            for &cell in &self.scratch {
                grid.insert_in_cell(entity, cell);
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
