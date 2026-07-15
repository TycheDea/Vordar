// engine-physics — collision detection and physics integration
//
// Registers broadphase, narrowphase, and resolve systems into the App.
// Game code subscribes to CollisionStarted / CollisionEnded via EventBus.
//
// Usage in game/src/main.rs:
//   App::new().add_plugin(PhysicsPlugin)...run();

pub mod aabb;
pub mod broadphase;
pub mod cell_update;
pub mod narrowphase;
pub mod resolve;

use broadphase::{BroadphaseSystem, CandidatePairs};
use cell_update::CellUpdateSystem;
use engine_app::app::App;
use engine_app::dev_stats::DevStats;
use engine_app::plugin::Plugin;
use engine_app::scheduler::{Phase, System, SystemOrder};
use engine_core::spatial::SpatialGrid;
use engine_core::traits::Resources;
use engine_core::World;
use narrowphase::{ActivePairs, NarrowphaseSystem};
use resolve::CollisionResolveSystem;

/// Publishes collision pipeline counters to the F3 dev overlay.
struct PhysicsStatsSystem;

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

/// Registers all physics systems and inserts required resources.
pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SpatialGrid::new(10.0))
            .insert_resource(CandidatePairs::new())
            .insert_resource(ActivePairs::new())
            // Rebuild SpatialGrid right before broadphase reads it. Collision
            // runs at 60 Hz everywhere — a PostUpdate rebuild would be 10
            // Hz-stale on the server (PostUpdate = snapshot rate), which
            // projectile hit detection can't tolerate. Bonus: same-tick
            // spawns enter the grid immediately (SpawnFlush precedes
            // Collision).
            .add_system(CellUpdateSystem::new(), Phase::Collision,        SystemOrder::First)
            .add_system(BroadphaseSystem::new(), Phase::Collision,        SystemOrder::after::<CellUpdateSystem>())
            .add_system(NarrowphaseSystem::new(), Phase::Collision,        SystemOrder::after::<BroadphaseSystem>())
            // Resolve stub holds the slot; game systems run Default in this phase
            .add_system(CollisionResolveSystem,  Phase::CollisionResolve, SystemOrder::Default)
            // Dev overlay counters — after CellUpdate so pair counts are current
            .add_system(PhysicsStatsSystem,      Phase::PostUpdate,       SystemOrder::Last);
    }
}
