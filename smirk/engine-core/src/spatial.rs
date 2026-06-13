// SpatialGrid — O(1) average proximity queries
//
// Divides the world into a grid of cells. Each entity registers in the cell
// that contains its position. A radius query only checks the cells that
// could overlap the search circle — never all entities.
//
// Usage per frame:
//   grid.clear();
//   for (entity, transform) in world.query::<(&Transform,)>().iter() {
//       grid.insert(entity, transform.position);
//   }
//   let nearby = grid.query_radius(player_pos, 10.0);

use crate::components::GridCell;
use glam::Vec3;
use hecs::Entity;
use std::collections::HashMap;

pub struct SpatialGrid {
    cell_size: f32,
    cells: HashMap<GridCell, Vec<Entity>>,
}

impl SpatialGrid {
    pub fn new(cell_size: f32) -> Self {
        Self { cell_size, cells: HashMap::new() }
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }

    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    pub fn insert(&mut self, entity: Entity, pos: Vec3) {
        self.cells.entry(self.cell_of(pos)).or_default().push(entity);
    }

    /// Insert an entity directly into a known cell (used by CellUpdateSystem for
    /// multi-cell entities whose footprint is pre-computed from hitbox extents).
    pub fn insert_in_cell(&mut self, entity: Entity, cell: GridCell) {
        self.cells.entry(cell).or_default().push(entity);
    }

    /// Remove a single entity from a specific cell. Used during DespawnFlush.
    pub fn remove(&mut self, cell: GridCell, entity: Entity) {
        if let Some(entities) = self.cells.get_mut(&cell) {
            entities.retain(|&e| e != entity);
        }
    }

    pub fn query_cell(&self, cell: GridCell) -> &[Entity] {
        self.cells.get(&cell).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Append nearby entities into a caller-owned buffer. Preferred over `query_radius`
    /// in hot paths — the caller reuses the buffer across frames to avoid per-call allocation.
    pub fn query_radius_into(&self, center: Vec3, radius: f32, out: &mut Vec<Entity>) {
        let min_col = ((center.x - radius) / self.cell_size).floor() as i32;
        let max_col = ((center.x + radius) / self.cell_size).ceil()  as i32;
        let min_row = ((center.z - radius) / self.cell_size).floor() as i32;
        let max_row = ((center.z + radius) / self.cell_size).ceil()  as i32;

        for col in min_col..=max_col {
            for row in min_row..=max_row {
                if let Some(es) = self.cells.get(&GridCell { col, row }) {
                    out.extend_from_slice(es);
                }
            }
        }
    }

    pub fn query_radius(&self, center: Vec3, radius: f32) -> Vec<Entity> {
        let mut entities = Vec::new();
        self.query_radius_into(center, radius, &mut entities);
        entities
    }

    fn cell_of(&self, pos: Vec3) -> GridCell {
        GridCell {
            col: (pos.x / self.cell_size).floor() as i32,
            row: (pos.z / self.cell_size).floor() as i32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hecs::World;

    #[test]
    fn test_spatial_grid() {
        let mut grid = SpatialGrid::new(10.0);
        let mut world = World::new();
        let entity1 = world.spawn(());
        let entity2 = world.spawn(());
        let entity3 = world.spawn(());
        let entity4 = world.spawn(());
        let entity5 = world.spawn(());
        grid.insert(entity1, Vec3::new(0.0, 0.0, 0.0));
        grid.insert(entity2, Vec3::new(10.0, 0.0, 0.0));
        grid.insert(entity3, Vec3::new(10.0, 0.0, 10.0));
        grid.insert(entity4, Vec3::new(0.0, 0.0, 20.0));
        grid.insert(entity5, Vec3::new(20.0, 0.0, 0.0));
        let mut entities_found = grid.query_radius(Vec3::new(5.0, 0.0, 5.0), 5.0);
        entities_found.sort();
        let mut expected = vec![entity1, entity2, entity3];
        expected.sort();
        assert_eq!(entities_found, expected);
    }

    #[test]
    fn cell_of_uses_floor_for_both_axes() {
        let grid = SpatialGrid::new(10.0);
        // z=9.9 must map to row 0, not row 1 (ceil would give 1)
        assert_eq!(grid.cell_of(Vec3::new(9.9, 0.0, 9.9)), GridCell { col: 0, row: 0 });
        assert_eq!(grid.cell_of(Vec3::new(10.0, 0.0, 10.0)), GridCell { col: 1, row: 1 });
    }

    #[test]
    fn remove_takes_entity_out_of_cell() {
        let mut grid = SpatialGrid::new(10.0);
        let mut world = World::new();
        let e = world.spawn(());
        let pos = Vec3::new(3.0, 0.0, 3.0);
        grid.insert(e, pos);
        let cell = grid.cell_of(pos);
        grid.remove(cell, e);
        assert!(grid.query_radius(Vec3::new(3.0, 0.0, 3.0), 1.0).is_empty());
    }
}
