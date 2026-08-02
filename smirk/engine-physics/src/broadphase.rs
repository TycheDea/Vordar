// BroadphaseSystem — spatial grid candidate pair generation.
//
// For each entity with CellOccupant + Hitbox, checks every grid cell it occupies
// and pairs it with every other entity sharing that cell. Pairs are canonical
// (smaller entity first) and deduplicated across cells via a HashSet.
//
// Output: CandidatePairs resource, consumed by NarrowphaseSystem in the same phase.

use engine_app::scheduler::System;
use engine_core::components::{CellOccupant, Hitbox};
use engine_core::spatial::SpatialGrid;
use engine_core::traits::Resources;
use engine_core::World;
use hecs::Entity;
use std::collections::HashSet;

/// Candidate entity pairs output by broadphase. Cleared and rebuilt every frame.
pub struct CandidatePairs(pub Vec<(Entity, Entity)>);

impl Default for CandidatePairs {
    fn default() -> Self {
        Self::new()
    }
}

impl CandidatePairs {
    pub fn new() -> Self { Self(Vec::new()) }
}

pub struct BroadphaseSystem {
    seen:  HashSet<(Entity, Entity)>,
    pairs: Vec<(Entity, Entity)>,
}

impl Default for BroadphaseSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl BroadphaseSystem {
    pub fn new() -> Self { Self { seen: HashSet::new(), pairs: Vec::new() } }
}

impl System for BroadphaseSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // world and resources are separate borrows — iterate both without an intermediate collect.
        self.seen.clear();
        self.pairs.clear();

        {
            let grid = resources.expect::<SpatialGrid>();

            for (entity, occupant, _) in world.query::<(Entity, &CellOccupant, &Hitbox)>().iter() {
                for &cell in &occupant.cells {
                    for &other in grid.query_cell(cell) {
                        if other == entity { continue; }
                        let pair = if entity < other { (entity, other) } else { (other, entity) };
                        if self.seen.insert(pair) { self.pairs.push(pair); }
                    }
                }
            }
        } // grid borrow ends

        // Swap into CandidatePairs — narrowphase swaps back an empty vec for us to reuse.
        std::mem::swap(
            &mut resources.expect_mut::<CandidatePairs>().0,
            &mut self.pairs,
        );
    }
}
