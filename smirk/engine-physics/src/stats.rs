// PhysicsStatsSystem — publishes collision pipeline counters to the F3 dev overlay.

use crate::broadphase::CandidatePairs;
use crate::narrowphase::ActivePairs;
use engine_app::dev_stats::DevStats;
use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;

pub(crate) struct PhysicsStatsSystem;

impl System for PhysicsStatsSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let candidates = resources.get::<CandidatePairs>().map(|c| c.0.len()).unwrap_or(0);
        let active     = resources.get::<ActivePairs>().map(|a| a.0.len()).unwrap_or(0);
        if let Some(stats) = resources.get_mut::<DevStats>() {
            if stats.open {
                stats.set("broadphase pairs", candidates);
                stats.set("active collisions", active);
            }
        }
    }
}
